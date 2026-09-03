//! Remote desktop, by handoff. We never speak RDP - we write a `.rdp` file and let
//! the OS open it: mstsc on Windows, Windows App / Microsoft Remote Desktop on
//! macOS, xfreerdp on Linux. One format, three clients, no embedded FreeRDP.
//!
//! The `rdp://` URI scheme exists but is useless here: on macOS it launches the
//! client and drops every parameter, so you get an empty connection window.
//!
//! RDP has no ProxyJump, so a host behind a bastion is reached by forwarding a
//! local port over the ssh chain patchbay already knows how to build. That is the
//! thing Royal TS sells separately as Royal Server.

use std::collections::HashMap;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `.rdp` is line-based, so a newline in a host or username would inject further
/// directives - `alternate shell:s:` runs a program on connect. Nothing but plain
/// printable text gets written into that file.
pub fn is_safe(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control())
}

/// Deliberately minimal: extra keys would be us guessing at someone's security
/// posture. Drive and clipboard redirection stay at the client's own defaults.
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

/// Ask the OS for a free port by binding and letting go. Racy in theory; the
/// window is microseconds and ssh reports a bind failure if it loses.
pub fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free local port: {e}"))
}

/// Whether something already holds a port a forward is about to ask for. `accepts`
/// can't answer this - a session already carrying the same `-L` makes the port answer,
/// so the tunnel would look up while its own ssh sat there having failed to bind.
pub fn port_taken(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

fn accepts(port: u16, limit: Duration) -> bool {
    let addr = match format!("127.0.0.1:{port}").to_socket_addrs().ok().and_then(|mut a| a.next()) {
        Some(a) => a,
        None => return false,
    };
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
            // A connection whose own port is the one it dialled is TCP's loopback
            // self-connect, not a listener: the OS can hand an outgoing socket the very
            // ephemeral port it is aiming at, and `free_port` picks from that range. Left
            // in, a tunnel whose ssh never managed to bind occasionally reads as up, and
            // whatever dials it then talks to itself.
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

/// A tunnel with no local port to connect to: every forward in it is a `-R`, bound on
/// the far end. `ExitOnForwardFailure` makes ssh give up when a bind is refused, so
/// "still here a moment later" is the only answer available without asking the far end.
///
/// ponytail: a bind that fails after this is a tunnel quietly carrying nothing. Reading
/// ssh's stderr is the upgrade if that ever bites.
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
    /// `ssh -N -L 127.0.0.1:local:target -J chain entry`, held open until closed.
    pub fn open(
        &self, id: u32, jack: &str, args: &[String], local: u16, via: String,
    ) -> Result<(), String> {
        let child = Command::new("ssh")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start the tunnel: {e}"))?;

        let mut t = Tunnel { child, jack: jack.to_string(), local, via: via.clone() };
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

    /// Nothing should outlive the window.
    pub fn close_all(&self) {
        let ids: Vec<u32> = self.0.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.close(id);
        }
    }
}

/// Writes the file somewhere the client can read it. Contains a hostname and
/// maybe a username - never a secret.
pub fn write_file(name: &str, body: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join("patchbay");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let safe: String = name.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
    let path = dir.join(format!("{}.rdp", if safe.is_empty() { "device".into() } else { safe }));
    let mut f = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(body.as_bytes()).map_err(|e| format!("{e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newline_cannot_smuggle_in_another_directive() {
        // `alternate shell` runs a program on connect, so this is the one that matters
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

    /// The listener is bound here and held for the whole check, rather than asked for
    /// through `free_port`: that one lets go of the port before it returns, and other
    /// tests in this binary are asking the OS for ephemeral ports at the same time - so
    /// anything built on "this port is still mine a moment later" goes red on a
    /// collision far more often than on a bug. What is left is race-free, and it still
    /// fails if the self-connect guard in `accepts` is ever turned the wrong way round.
    #[test]
    fn accepts_sees_something_listening_and_free_port_gives_a_plausible_one() {
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = held.local_addr().unwrap().port();
        assert!(accepts(p, Duration::from_millis(600)), "a real listener answers");
        assert!(free_port().unwrap() > 1024);
    }

    /// The readiness check a remote-only forward gets: ssh that died is a tunnel that
    /// never came up, and there is no local port to prove it either way.
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
