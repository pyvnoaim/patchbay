//! Remote desktop by handoff: a `.rdp` file for the system client, and the ssh tunnels
//! that carry RDP and VNC through a jump chain. The `rdp://` scheme drops every
//! parameter on macOS, which is why it is a file.

use std::collections::HashMap;
use std::io::{BufRead, Write};
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
/// `ExitOnForwardFailure` exits on a refused bind, so still running means up. Waits
/// for it to be up rather than a flat sleep, so a fast exit answers fast.
fn still_running(child: &mut Child, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if !matches!(child.try_wait(), Ok(None)) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// What ssh said, kept for the error a failed tunnel shows. Read on its own thread:
/// a pipe nobody drains blocks ssh once it fills.
type Said = Arc<Mutex<Vec<String>>>;

fn read_stderr(child: &mut Child) -> Said {
    let said: Said = Arc::default();
    if let Some(err) = child.stderr.take() {
        let sink = said.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                let mut s = sink.lock().unwrap();
                // The last few lines are the reason; warnings above them are not.
                if s.len() == 8 {
                    s.remove(0);
                }
                s.push(line);
            }
        });
    }
    said
}

/// ssh's own words for a failure, after its noise. Empty when it said nothing.
fn last_word(said: &Said) -> String {
    said.lock()
        .unwrap()
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty() && !l.starts_with("Warning: Permanently added"))
        .cloned()
        .unwrap_or_default()
}

pub struct Tunnel {
    child: Child,
    said: Said,
    pub jack: String,
    pub local: u16,
    pub via: String,
}

/// A live tunnel as the window sees it: id, jack, local port, route.
pub type Live = (u32, String, u16, String);
/// A tunnel found dead: jack, and ssh's last line.
pub type Ended = (String, String);

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
        let mut child = Command::new("ssh")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the tunnel: {e}"))?;
        let said = read_stderr(&mut child);

        let mut t = Tunnel {
            child,
            said,
            jack: jack.to_string(),
            local,
            via: via.clone(),
        };
        let up = match local {
            0 => still_running(&mut t.child, Duration::from_millis(1500)),
            p => accepts(p, Duration::from_secs(12)),
        };
        if !up {
            let _ = t.child.kill();
            let _ = t.child.wait();
            let why = last_word(&t.said);
            return Err(if why.is_empty() {
                format!("the tunnel to {jack} never came up - check you can reach {via}")
            } else {
                format!("the tunnel to {jack} never came up: {why}")
            });
        }
        self.0.lock().unwrap().insert(id, t);
        Ok(())
    }

    /// Live tunnels, and the ones found dead on the way: an ssh that exited since the
    /// check (a bind refused later, the far end gone) is dropped and reported with its
    /// last line, so the list never shows a port nothing is behind.
    pub fn list(&self) -> (Vec<Live>, Vec<Ended>) {
        let mut held = self.0.lock().unwrap();
        let dead: Vec<u32> = held
            .iter_mut()
            .filter_map(|(id, t)| (!matches!(t.child.try_wait(), Ok(None))).then_some(id))
            .copied()
            .collect();
        let ended = dead
            .into_iter()
            .filter_map(|id| held.remove(&id))
            .map(|mut t| {
                let _ = t.child.wait();
                (t.jack, last_word(&t.said))
            })
            .collect();
        let live = held
            .iter()
            .map(|(id, t)| (*id, t.jack.clone(), t.local, t.via.clone()))
            .collect();
        (live, ended)
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
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Hand the `.rdp` to the system client. On Linux xfreerdp is tried by name first:
/// it takes a `.rdp` file as its argument but registers no MIME type, so `xdg-open`
/// only finds it through Remmina, and otherwise lands the file in a text editor.
/// `/cert:tofu` because xfreerdp asks about an unknown certificate on its terminal,
/// and spawned from here it has none; first-use pinning is what `rdp_session.rs`
/// does with `rdp_known_hosts` too, and a changed certificate is still refused.
pub fn hand_off(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    for bin in ["xfreerdp3", "xfreerdp"] {
        if Command::new(bin)
            .arg(path)
            .arg("/cert:tofu")
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    crate::commands::os_open(path.as_os_str())
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
        assert!(!still_running(&mut dead, Duration::from_millis(1200)));
        let mut alive = Command::new("sh").arg("-c").arg("sleep 5").spawn().unwrap();
        assert!(still_running(&mut alive, Duration::from_millis(300)));
        let _ = alive.kill();
    }

    #[test]
    #[cfg(not(windows))]
    fn a_failed_tunnel_says_what_ssh_said() {
        let mut child = Command::new("sh")
            .args(["-c", "echo 'Warning: Permanently added x' >&2; echo 'bind: Address already in use' >&2; exit 1"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let said = read_stderr(&mut child);
        let _ = child.wait();
        // The reader thread is a step behind the exit; give it a moment, not a guess.
        let deadline = Instant::now() + Duration::from_secs(2);
        while last_word(&said).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(last_word(&said), "bind: Address already in use");
    }
}
