//! Files, by handing them to `/usr/bin/sftp`: stateless batch runs that share one ssh
//! connection through multiplexing, so only the first one authenticates.

use crate::patchbay;
use serde::Serialize;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// For listings only. A transfer takes as long as the file is big, so it relies on
/// ssh's keepalive from `mux` instead.
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// As the server printed it; reformatting another locale's date gets it wrong.
    pub modified: String,
}

/// One control socket per jack. A stale one is harmless: ssh only reuses a socket a
/// live master still answers on. The name is hashed to a fixed length: a socket path
/// caps at 104 bytes on macOS, `$TMPDIR` there is already ~50, and ssh adds 17 more
/// while it sets the socket up - a long device name made every session exit 255.
/// `$XDG_RUNTIME_DIR` first: Linux's temp dir is the shared `/tmp`, where another user
/// could put a socket at this predictable name before ours and have ssh talk to it.
pub fn control_path(name: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    name.hash(&mut h);
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("pb-{}-{:016x}", std::process::id(), h.finish()))
}

/// A jack's name as one path segment; it comes from a file someone else may have written.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Where a file opened for editing lives. Per jack, or two devices with the same file
/// name would share one copy.
pub fn edit_dir(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("patchbay-edit")
        .join(safe_name(name))
}

/// ssh's argv as sftp wants it: the port is `-P` not `-p`, every option must come before
/// the destination (so `-b -` goes in here, not appended), and forwards are dropped
/// because re-binding a port a live session holds only produces an error.
fn sftp_args(ssh: Vec<String>, socket: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(ssh.len() + 8);
    let mut it = ssh.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-p" => {
                out.push("-P".into());
                out.extend(it.next());
            }
            f if patchbay::FORWARD_FLAGS.contains(&f) => {
                it.next();
            }
            _ => out.push(a),
        }
    }
    let dest = out.pop();
    // The batch is on stdin and stops at the first failure.
    let batch = ["-b".to_string(), "-".into()];
    mux(socket)
        .into_iter()
        .chain(batch)
        .chain(out)
        .chain(dest)
        .collect()
}

/// The options that share one ssh connection. `pty.rs` puts the same ones on a terminal
/// session, so a shell authenticates and the file browser rides it. Win32-OpenSSH has
/// no multiplexing, so only the keepalives go on Windows.
/// ponytail: ControlPersist=60 outlives `close_all()`; `ControlPersist=no` plus `-O exit`
/// if that ever matters.
pub fn mux(socket: &Path) -> Vec<String> {
    let _ = socket;
    let mut opts = vec![
        // How a transfer notices a dead host: a minute of silence, not a wall clock.
        "-o".to_string(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=4".into(),
    ];
    #[cfg(not(windows))]
    opts.extend([
        "-o".to_string(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", socket.display()),
        "-o".into(),
        "ControlPersist=60".into(),
    ]);
    opts
}

/// A local path for the batch line. `\` is sftp's escape character inside quotes, so
/// a Windows path is rewritten with `/`, which Windows accepts.
fn local_arg(p: &Path) -> Result<String, String> {
    let text = p
        .to_str()
        .ok_or("that path isn't text sftp can be given")?
        .replace('\\', "/");
    safe_path(&text)?;
    Ok(text)
}

fn args_for(name: &str) -> Result<Vec<String>, String> {
    let jacks = crate::commands::load_jacks()?;
    let resolved = patchbay::resolve(name, &jacks)?;
    let ssh = patchbay::ssh_args(&resolved, &jacks)?;
    Ok(sftp_args(ssh, &control_path(&resolved)))
}

/// Run a batch of sftp commands and return stdout. A transfer passes no deadline and
/// leans on ssh's keepalive, or a large download would be killed halfway.
fn batch(name: &str, script: &str, deadline: Option<Duration>) -> Result<String, String> {
    let mut child = Command::new("sftp")
        .args(args_for(name)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run sftp: {e}"))?;

    child
        .stdin
        .take()
        .ok_or("sftp took no input")?
        .write_all(script.as_bytes())
        .map_err(|e| e.to_string())?;

    let started = std::time::Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if deadline.is_some_and(|d| started.elapsed() >= d) => {
                let _ = child.kill();
                let secs = deadline.unwrap_or_default().as_secs();
                return Err(format!("{name} stopped answering after {secs}s"));
            }
            None => std::thread::sleep(Duration::from_millis(30)),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.is_empty() {
        return Err(format!("sftp failed ({})", out.status));
    }
    // A batch has no terminal to type a password into; a terminal session to the same
    // device leaves behind a connection this can share, except on Windows.
    if !err.contains("Permission denied") {
        return Err(err);
    }
    #[cfg(not(windows))]
    let hint =
        format!("connect a terminal to {name} first, and the file browser will use that session");
    #[cfg(windows)]
    let hint = format!("{name} wants a password, and sftp has no terminal to ask in - this one needs a key or your agent");
    Err(format!("{err}\n\nno key would do - {hint}"))
}

/// One `ls -l` line into an entry. The format is the server's, so anything unreadable
/// is skipped rather than guessed at.
pub fn parse_line(line: &str) -> Option<Entry> {
    let f: Vec<&str> = line.split_whitespace().collect();
    // mode links owner group size month day time name: nine fields at least.
    if f.len() < 9 || f[0].len() < 10 {
        return None;
    }
    // The name is everything from the ninth field on. Walked to, not searched for:
    // `line.find` would hit the owner column for a file called `root`.
    let mut rest = line.trim_start();
    for _ in 0..8 {
        rest = rest.split_once(char::is_whitespace)?.1.trim_start();
    }
    // A symlink prints as `name -> target`.
    let name = match f[0].starts_with('l') {
        true => rest.split(" -> ").next()?.trim_end(),
        false => rest.trim_end(),
    }
    .to_string();
    // The far end wrote this and every caller appends it to a directory, so a slash
    // or `..` would walk out of that directory, and out of ~/Downloads on the way back.
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return None;
    }
    Some(Entry {
        dir: f[0].starts_with('d'),
        size: f[4].parse().unwrap_or(0),
        modified: format!("{} {} {}", f[5], f[6], f[7]),
        name,
    })
}

/// A path fit for a batch line, local or remote. The batch language is line-based and
/// quotes with `"`, so a newline or quote would start another command. Refused, not
/// escaped, like a newline in a `.rdp`.
fn safe_path(path: &str) -> Result<&str, String> {
    match path.contains(['\n', '\r', '"', '\\', '\0']) {
        true => Err(format!(
            "\"{path}\" has a character sftp can't be given safely"
        )),
        false => Ok(path),
    }
}

/// The path comes from the server's `pwd`, not from appending to the one sent: only
/// the server can resolve `..` and symlinks.
#[derive(Debug, Serialize, PartialEq)]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
}

pub fn ls(name: &str, path: &str) -> Result<Listing, String> {
    let dir = safe_path(path)?;
    let out = batch(name, &format!("cd \"{dir}\"\npwd\nls -la\n"), Some(TIMEOUT))?;
    Ok(parse_listing(&out, dir))
}

fn parse_listing(out: &str, asked_for: &str) -> Listing {
    let path = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("Remote working directory: "))
        .map_or_else(|| asked_for.to_string(), |p| p.trim().to_string());
    Listing {
        path,
        entries: out.lines().filter_map(parse_line).collect(),
    }
}

/// Whether a shared connection is already up. The control socket exists only once ssh
/// has authenticated, so this asks nothing of the far end.
pub fn ready(name: &str) -> Result<bool, String> {
    let jacks = crate::commands::load_jacks()?;
    let resolved = patchbay::resolve(name, &jacks)?;
    Ok(control_path(&resolved).exists())
}

pub fn downloads() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Downloads")
}

/// Download a file, or a folder with `recurse` (sftp's own `get -r`).
pub fn get(name: &str, remote: &str, into: &Path, recurse: bool) -> Result<PathBuf, String> {
    let from = safe_path(remote)?;
    let leaf = from
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("no file name in that path")?;
    // This decides what gets created, and on failure removed, inside the download
    // folder. `..` would be its parent.
    if leaf == "." || leaf == ".." {
        return Err(format!("\"{remote}\" doesn't name a file"));
    }
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    let to = into.join(leaf);
    let dest = local_arg(&to)?;
    // A failed transfer leaves a partial file that looks like a download that worked.
    let flag = if recurse { " -r" } else { "" };
    batch(name, &format!("get{flag} \"{from}\" \"{dest}\"\n"), None).inspect_err(|_| {
        let _ = match recurse {
            true => std::fs::remove_dir_all(&to),
            false => std::fs::remove_file(&to),
        };
    })?;
    Ok(to)
}

/// Make, rename or remove, one batch line each. Any other verb is refused, not sent.
pub fn edit(name: &str, op: &str, path: &str, to: &str) -> Result<(), String> {
    let p = safe_path(path)?;
    let line = match op {
        "mkdir" => format!("mkdir \"{p}\"\n"),
        "rm" => format!("rm \"{p}\"\n"),
        "rmdir" => format!("rmdir \"{p}\"\n"),
        "rename" => format!("rename \"{p}\" \"{}\"\n", safe_path(to)?),
        _ => return Err(format!("no file operation named \"{op}\"")),
    };
    match batch(name, &line, Some(TIMEOUT)) {
        // sftp has no recursive remove; a non-empty folder answers with the bare word
        // "Failure".
        Err(e) if op == "rmdir" && e.contains("Failure") => Err(format!(
            "\"{p}\" isn't empty, and sftp only removes an empty folder"
        )),
        other => other.map(|_| ()),
    }
}

pub fn put(name: &str, local: &Path, remote_dir: &str) -> Result<(), String> {
    let dir = safe_path(remote_dir)?;
    let from = local_arg(local)?;
    let leaf = local
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("that file has no usable name")?;
    safe_path(leaf)?;
    batch(name, &format!("put \"{from}\" \"{dir}/{leaf}\"\n"), None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_device_name_still_fits_a_socket_path() {
        let long = control_path(&"SRV-Kerio (Ubuntu) ".repeat(20));
        let short = control_path("a");
        assert_eq!(long.as_os_str().len(), short.as_os_str().len());
        assert_ne!(long, short);
    }

    #[test]
    fn a_listing_line_becomes_an_entry_and_junk_is_skipped() {
        let dir =
            parse_line("drwxr-xr-x    5 root     root         4096 Jan 14 09:31 etc").unwrap();
        assert!(dir.dir);
        assert_eq!(dir.name, "etc");

        let file =
            parse_line("-rw-r--r--    1 deploy   deploy      12345 Mar  2 2024 notes.txt").unwrap();
        assert!(!file.dir);
        assert_eq!(file.size, 12345);
        assert_eq!(file.name, "notes.txt");

        // Names contain spaces, and a listing carries lines that aren't entries.
        assert_eq!(
            parse_line("-rw-r--r--  1 a b 7 Jan  1 00:00 two words.conf")
                .unwrap()
                .name,
            "two words.conf"
        );
        assert!(
            parse_line("drwxr-xr-x 2 a b 4096 Jan  1 00:00 .").is_none(),
            "\".\" is not a file"
        );
        // A file named after its owner must not take the name from the owner column.
        assert_eq!(
            parse_line("-rw-r--r-- 1 root root 7 Jan  1 00:00 root")
                .unwrap()
                .name,
            "root"
        );
        // A symlink's target is not part of its name.
        assert_eq!(
            parse_line("lrwxrwxrwx 1 a b 11 Jan  1 00:00 latest -> /var/log/x")
                .unwrap()
                .name,
            "latest"
        );
        // macOS's sftp-server prints "?" for the link count and pads its columns.
        let real =
            parse_line("-rw-r--r--    ? alice staff       98768 Sep  1 13:30 #76 report.pdf")
                .unwrap();
        assert_eq!(real.name, "#76 report.pdf");
        assert_eq!(real.size, 98768);
        assert!(parse_line("sftp> cd /etc").is_none());
        assert!(parse_line("").is_none());
    }

    /// A hostile name would walk out of the directory, and a failed download would then
    /// remove what it thinks it created there.
    #[test]
    fn a_listing_name_is_a_name_and_never_a_path() {
        for hostile in [
            "-rw-r--r-- 1 a b 7 Jan  1 00:00 ../../.ssh/authorized_keys",
            "-rw-r--r-- 1 a b 7 Jan  1 00:00 sub/..",
            "drwxr-xr-x 2 a b 4096 Jan  1 00:00 ..",
        ] {
            assert!(
                parse_line(hostile).is_none(),
                "{hostile:?} was taken as a file name"
            );
        }
        assert!(
            get("nowhere", "/tmp/..", Path::new("/tmp"), true).is_err(),
            "\"..\" is a folder, not a file"
        );
    }

    #[test]
    fn a_listing_says_where_it_actually_is() {
        let out = "sftp> cd \"./Desktop/..\"\n\
                   Remote working directory: /Users/me\n\
                   drwxr-xr-x 5 me staff 160 Jan 14 09:31 Downloads\n";
        let l = parse_listing(out, "./Desktop/..");
        assert_eq!(l.path, "/Users/me");
        assert_eq!(l.entries.len(), 1, "the pwd line is not a file");
        assert_eq!(l.entries[0].name, "Downloads");

        // A server that doesn't echo pwd falls back to the path asked for.
        assert_eq!(parse_listing("", "/var/log").path, "/var/log");
    }

    /// sftp stops reading options at the destination.
    #[test]
    fn every_option_lands_before_the_destination() {
        let ssh = [
            "-J",
            "bastion",
            "-p",
            "2222",
            "-L",
            "8080:localhost:80",
            "-D",
            "1080",
            "me@nas",
        ];
        let a = sftp_args(
            ssh.iter().map(|s| s.to_string()).collect(),
            Path::new("/tmp/sock"),
        );

        assert_eq!(a.last().unwrap(), "me@nas", "the destination comes last");
        assert!(
            a.windows(2).any(|w| w == ["-P", "2222"]),
            "sftp spells the port -P: {a:?}"
        );
        assert!(!a
            .iter()
            .any(|x| x == "-p" || x == "-L" || x == "8080:localhost:80"));
        assert!(
            !a.iter().any(|x| x == "-D" || x == "1080"),
            "every kind of forward goes: {a:?}"
        );
        assert!(
            a.windows(2).any(|w| w == ["-J", "bastion"]),
            "the jump chain survives"
        );
        assert!(
            a.windows(2).any(|w| w == ["-b", "-"]),
            "the batch flag is here, not appended"
        );
        assert!(
            a.windows(2).any(|w| w == ["-o", "ServerAliveInterval=15"]),
            "a dead host is noticed"
        );
    }

    #[test]
    fn a_path_that_could_smuggle_a_command_is_refused() {
        for bad in [
            "/tmp/x\nrm -rf /",
            "/tmp/\"; get /etc/shadow",
            "/tmp/a\\b",
            "/tmp/a\rb",
        ] {
            assert!(safe_path(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(safe_path("/var/log/syslog").is_ok());
        assert!(safe_path("/home/me/two words.txt").is_ok());

        // A Windows path is rewritten rather than refused.
        assert_eq!(
            local_arg(Path::new(r"C:\Users\me\notes.txt")).unwrap(),
            "C:/Users/me/notes.txt"
        );

        let dropped = std::env::temp_dir().join("a\"\nget /etc/shadow");
        assert!(
            put("nowhere", &dropped, "/tmp").is_err(),
            "a local name can smuggle too"
        );

        assert!(edit("nowhere", "rename", "/tmp/a", "/tmp/b\nrm /etc/passwd").is_err());
        assert!(edit("nowhere", "mkdir", "/tmp/x\"; rm -rf /", "").is_err());
        assert!(edit("nowhere", "chmod 777", "/tmp/a", "").is_err());
    }
}
