//! Remote desktop by handoff: a `.rdp` file for the system client, and the ssh tunnels
//! that carry RDP and VNC through a jump chain. The `rdp://` scheme drops every
//! parameter on macOS, which is why it is a file.

use std::collections::HashMap;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `.rdp` is line-based, so a control character in a host or username would inject a
/// directive, and `alternate shell:s:` runs a program on connect.
pub fn is_safe(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control())
}

/// Minimal on purpose: drive and clipboard redirection stay at the client's defaults.
pub fn rdp_file(addr: &str, user: Option<&str>) -> Result<String, String> {
    if !is_safe(addr) {
        return Err(format!("\"{addr}\" isn't a usable address"));
    }
    let mut out = format!("full address:s:{addr}\r\nprompt for credentials:i:1\r\n");
    if let Some(u) = user.map(str::trim).filter(|u| !u.is_empty()) {
        if !is_safe(u) {
            return Err(format!("\"{u}\" isn't a usable username"));
        }
        out.push_str(&format!("username:s:{u}\r\n"));
    }
    Ok(out)
}

/// Bind and let go. Racy in theory; ssh reports a bind failure if it loses.
pub fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free local port: {e}"))
}

/// Whether a port is already held. `accepts` can't tell: a session carrying the same
/// `-L` makes the port answer while the new ssh has failed to bind.
pub fn port_taken(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

fn accepts(port: u16, limit: Duration) -> bool {
    let addr = match format!("127.0.0.1:{port}")
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
    {
        Some(a) => a,
        None => return false,
    };
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
            // A connection whose own port is the one it dialled is TCP's loopback
            // self-connect, not a listener; without this a tunnel that never bound reads as up.
            let mine = s.local_addr().ok().map(|a| a.port());
            let _ = s.shutdown(Shutdown::Both);
            if mine != Some(port) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// The readiness check for a tunnel with no local port (all `-R`): ssh with
/// `ExitOnForwardFailure` exits on a refused bind, so still running means up.
/// ponytail: a bind that fails later carries nothing; reading ssh's stderr is the upgrade.
fn still_running(child: &mut Child) -> bool {
    std::thread::sleep(Duration::from_millis(1200));
    matches!(child.try_wait(), Ok(None))
}

pub struct Tunnel {
    child: Child,
    pub jack: String,
    pub local: u16,
    pub via: String,
}

#[derive(Default)]
pub struct Tunnels(Mutex<HashMap<u32, Tunnel>>);
pub type SharedTunnels = Arc<Tunnels>;

impl Tunnels {
    /// Spawn `ssh -N ...` and hold it until closed. `local` 0 means no port to watch.
    pub fn open(
        &self,
        id: u32,
        jack: &str,
        args: &[String],
        local: u16,
        via: String,
    ) -> Result<(), String> {
        let child = Command::new("ssh")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the tunnel: {e}"))?;

        let mut t = Tunnel {
            child,
            jack: jack.to_string(),
            local,
            via: via.clone(),
        };
        let up = match local {
            0 => still_running(&mut t.child),
            p => accepts(p, Duration::from_secs(12)),
        };
        if !up {
            let _ = t.child.kill();
            let _ = t.child.wait();
            return Err(format!(
                "the tunnel to {jack} never came up - check you can reach {via}"
            ));
        }
        self.0.lock().unwrap().insert(id, t);
        Ok(())
    }

    pub fn list(&self) -> Vec<(u32, String, u16, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(id, t)| (*id, t.jack.clone(), t.local, t.via.clone()))
            .collect()
    }

    pub fn close(&self, id: u32) {
        if let Some(mut t) = self.0.lock().unwrap().remove(&id) {
            let _ = t.child.kill();
            let _ = t.child.wait();
        }
    }

    /// Called on exit: `ssh -N` has no parent to hang up on and would keep its ports.
    pub fn close_all(&self) {
        let ids: Vec<u32> = self.0.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.close(id);
        }
    }
}

/// Write the file where the client can read it. It holds a host and maybe a user,
/// never a secret.
pub fn write_file(name: &str, body: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join("patchbay");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let safe: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let path = dir.join(format!(
        "{}.rdp",
        if safe.is_empty() {
            "device".into()
        } else {
            safe
        }
    ));
    let mut f = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(body.as_bytes()).map_err(|e| format!("{e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newline_cannot_smuggle_in_another_directive() {
        // `alternate shell` runs a program on connect.
        let attack = "10.0.0.5\r\nalternate shell:s:calc.exe";
        assert!(rdp_file(attack, None).is_err());
        assert!(rdp_file("10.0.0.5", Some("bob\nfull address:s:evil")).is_err());
        assert!(rdp_file("10.0.0.5:3389", Some("bob")).is_ok());
        assert!(rdp_file("", None).is_err());
    }

    #[test]
    fn the_file_carries_the_address_and_user_and_nothing_else() {
        let f = rdp_file("10.0.0.5:3389", Some("administrator")).unwrap();
        assert!(f.contains("full address:s:10.0.0.5:3389"));
        assert!(f.contains("username:s:administrator"));
        assert!(!f.to_lowercase().contains("password"), "we never write one");
        // an omitted username must not leave an empty directive behind
        assert!(!rdp_file("h", Some("  ")).unwrap().contains("username"));
    }

    /// The listener is held for the whole check rather than taken from `free_port`,
    /// which lets go before returning and races other tests for ephemeral ports.
    #[test]
    fn accepts_sees_something_listening_and_free_port_gives_a_plausible_one() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = held.local_addr().unwrap().port();
        assert!(
            accepts(p, Duration::from_millis(600)),
            "a real listener answers"
        );
        assert!(free_port().unwrap() > 1024);
    }

    #[test]
    #[cfg(not(windows))]
    fn a_tunnel_with_no_local_port_is_judged_by_ssh_still_being_alive() {
        let mut dead = Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap();
        assert!(!still_running(&mut dead));
        let mut alive = Command::new("sh").arg("-c").arg("sleep 5").spawn().unwrap();
        assert!(still_running(&mut alive));
        let _ = alive.kill();
    }
}
