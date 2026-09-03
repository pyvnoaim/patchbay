//! Files over the same connection, by handing them to `/usr/bin/sftp`.
//!
//! The rule that governs everything else here governs this: we do not implement a
//! protocol. `sftp` is what ssh ships, so the agent, `~/.ssh/config` and `known_hosts`
//! keep doing the work, and a jump chain is the one `patchbay.rs` already worked out.
//!
//! Every operation is its own `sftp -b -` run, which would be a fresh handshake each
//! time - so they share one through ssh's own multiplexing. That is why this is a set
//! of stateless commands rather than a long-lived process with a prompt to parse.

use crate::patchbay;
use serde::Serialize;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Long enough for a big directory over a slow link, short enough that a host which
/// stopped answering doesn't hold a tab open forever. A *listing* only - a transfer
/// takes as long as the file is big, and a clock is the wrong way to notice a dead
/// host mid-copy. `mux` sets ssh's own keepalive for that.
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// As the server printed it - the format is the server's business, and reformatting
    /// someone else's locale into ours is how you get a date that's wrong by a year.
    pub modified: String,
}

/// One control socket per jack, so a directory listing after the first is not another
/// authentication. Named by the jack, in the OS temp dir: a stale one is harmless
/// because ssh only reuses a socket a live master is still answering on.
pub fn control_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("patchbay-sftp-{}-{}", std::process::id(), safe_name(name)))
}

/// A jack's name as one path segment. It comes from a file a colleague may have
/// written, and both callers build a path out of it.
fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Where a file opened for editing is kept while it is open. Per jack, or two devices
/// with a `docker-compose.yml` would be editing the same copy.
pub fn edit_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join("patchbay-edit").join(safe_name(name))
}

/// `sftp` takes the same options as `ssh` bar two: the port is `-P`, not `-p`, and it
/// is getopt-strict - every option has to come *before* the destination, which is what
/// `ssh_args` puts last. So `-b -` goes in here rather than being appended by the
/// caller; appended, sftp answers a directory listing with its usage message.
/// Forwards of every kind are dropped: a file copy has no use for the hop's tunnels,
/// and re-binding a port a live session already holds only produces an error.
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
    // The batch is on stdin and stops at the first failure, which is what makes an
    // error mean something.
    let batch = ["-b".to_string(), "-".into()];
    mux(socket).into_iter().chain(batch).chain(out).chain(dest).collect()
}

/// Share one ssh connection. `pty.rs` puts the same options on a terminal session, so
/// opening a shell and then browsing files is one authentication and one connection -
/// which is what "files over the same connection" has to mean when `-b` rules out
/// asking for a password.
///
/// Win32-OpenSSH has no connection multiplexing, so there is nothing to share there and
/// the options are left off rather than sent to be ignored. The keepalives are the
/// half that works everywhere.
///
/// ponytail: ControlPersist keeps the connection up for 60s after the last user of
/// it exits, so `close_all()` on window exit doesn't reach it. It expires itself;
/// give it `ControlPersist=no` and an explicit `-O exit` if that ever matters.
pub fn mux(socket: &Path) -> Vec<String> {
    let _ = socket;
    let mut opts = vec![
        // How a transfer notices the far end has gone: ssh asks, and gives up after a
        // minute of silence. A wall clock can't tell a dead host from a large file.
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

/// A local path, as sftp's batch language will read it. Windows spells its paths with
/// `\`, which is the escape character inside a quoted argument - `C:\Users` would
/// arrive as `C:Users`. Windows takes `/` in a path perfectly well, so that is what
/// goes in the line.
fn local_arg(p: &Path) -> Result<String, String> {
    let text = p
        .to_str()
        .ok_or("that path isn't text sftp can be given")?
        .replace('\\', "/");
    safe_path(&text)?;
    Ok(text)
}

fn args_for(name: &str) -> Result<Vec<String>, String> {
    let jacks = patchbay::load_all(&patchbay::config_path())?;
    let resolved = patchbay::resolve(name, &jacks)?;
    let ssh = patchbay::ssh_args(&resolved, &jacks)?;
    Ok(sftp_args(ssh, &control_path(&resolved)))
}

/// Runs a batch of sftp commands, returning stdout. `deadline` is for the operations
/// that should be quick; a transfer passes `None` and leans on ssh's keepalive, or a
/// download that is merely large gets killed halfway through.
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
    // A batch has no terminal to type a password into, so a host that wants one can
    // only be reached through a connection something else already authenticated -
    // which is exactly what a terminal session to the same device leaves behind. On
    // Windows there is no such sharing, so saying so would be advice that can't work.
    if !err.contains("Permission denied") {
        return Err(err);
    }
    #[cfg(not(windows))]
    let hint = format!("connect a terminal to {name} first, and the file browser will use that session");
    #[cfg(windows)]
    let hint = format!("{name} wants a password, and sftp has no terminal to ask in - this one needs a key or your agent");
    Err(format!("{err}\n\nno key would do - {hint}"))
}

/// One `ls -l` line into an entry. The format is the server's `ls`, so this is
/// deliberately forgiving: anything it can't read is skipped rather than guessed at.
pub fn parse_line(line: &str) -> Option<Entry> {
    let f: Vec<&str> = line.split_whitespace().collect();
    // mode links owner group size month day time name - nine at the very least.
    if f.len() < 9 || f[0].len() < 10 {
        return None;
    }
    // The name is everything from the ninth field on, because names contain spaces.
    // Walked to rather than searched for: `line.find(f[8])` finds the *first* time
    // that text appears, which is the owner column for a file called `root`.
    let mut rest = line.trim_start();
    for _ in 0..8 {
        rest = rest.split_once(char::is_whitespace)?.1.trim_start();
    }
    // A symlink is printed `name -> target`, and the target is not part of the name.
    let name = match f[0].starts_with('l') {
        true => rest.split(" -> ").next()?.trim_end(),
        false => rest.trim_end(),
    }
    .to_string();
    // A name is a name, never a path: the far end wrote this, and every caller
    // appends it to the directory it came from. One with a slash in it would walk
    // out of that directory - and out of ~/Downloads on the way back down.
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

/// A path we are willing to put in a batch script - either side of the line. sftp's
/// batch language is line-based and quotes with `"`, so a newline or a quote in a path
/// would end the command and start another - the same hole as a newline in a `.rdp`,
/// closed the same way: refuse rather than escape. Local names get the same check as
/// remote ones: a file dragged in is named by whoever made it, and macOS and Linux both
/// allow a quote in a filename.
fn safe_path(path: &str) -> Result<&str, String> {
    match path.contains(['\n', '\r', '"', '\\', '\0']) {
        true => Err(format!("\"{path}\" has a character sftp can't be given safely")),
        false => Ok(path),
    }
}

/// Where we ended up as well as what is there. The path comes back from `pwd` rather
/// than from appending to the one we sent: walking in and out of folders otherwise
/// builds `./Desktop/../Downloads/../Downloads`, and only the server can resolve a
/// symlink or say what `.` was.
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
    Listing { path, entries: out.lines().filter_map(parse_line).collect() }
}

/// Whether a connection to this jack is already open - the control socket exists only
/// once ssh has authenticated, so this is "would a listing work now" without opening
/// anything or asking the far end.
pub fn ready(name: &str) -> Result<bool, String> {
    let jacks = patchbay::load_all(&patchbay::config_path())?;
    let resolved = patchbay::resolve(name, &jacks)?;
    Ok(control_path(&resolved).exists())
}

/// Where the user actually looks for a file they just fetched.
pub fn downloads() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Downloads")
}

/// `recurse` is a folder: sftp's own `get -r`, rather than us walking the tree and
/// issuing one `get` per file.
pub fn get(name: &str, remote: &str, into: &Path, recurse: bool) -> Result<PathBuf, String> {
    let from = safe_path(remote)?;
    let leaf = from.rsplit('/').next().filter(|s| !s.is_empty()).ok_or("no file name in that path")?;
    // Belt to `parse_line`'s braces: this decides what gets created - and, if the
    // transfer fails, removed - inside the download folder. `..` would be its parent.
    if leaf == "." || leaf == ".." {
        return Err(format!("\"{remote}\" doesn't name a file"));
    }
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
    let to = into.join(leaf);
    let dest = local_arg(&to)?;
    // A transfer that fails leaves as much of the file as it managed, which is worse
    // than nothing: it looks like a download that worked.
    let flag = if recurse { " -r" } else { "" };
    batch(name, &format!("get{flag} \"{from}\" \"{dest}\"\n"), None).inspect_err(|_| {
        let _ = match recurse {
            true => std::fs::remove_dir_all(&to),
            false => std::fs::remove_file(&to),
        };
    })?;
    Ok(to)
}

/// The writes a file list needs, as the one batch line each of them is. A verb the far
/// end was never going to be asked for is refused here rather than sent.
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
        // sftp has no recursive remove, and a server's answer for a folder that still
        // has something in it is the bare word "Failure".
        Err(e) if op == "rmdir" && e.contains("Failure") => {
            Err(format!("\"{p}\" isn't empty, and sftp only removes an empty folder"))
        }
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
    fn a_listing_line_becomes_an_entry_and_junk_is_skipped() {
        let dir = parse_line("drwxr-xr-x    5 root     root         4096 Jan 14 09:31 etc").unwrap();
        assert!(dir.dir);
        assert_eq!(dir.name, "etc");

        let file = parse_line("-rw-r--r--    1 deploy   deploy      12345 Mar  2 2024 notes.txt").unwrap();
        assert!(!file.dir);
        assert_eq!(file.size, 12345);
        assert_eq!(file.name, "notes.txt");

        // Names contain spaces, and a listing carries lines that aren't entries.
        assert_eq!(
            parse_line("-rw-r--r--  1 a b 7 Jan  1 00:00 two words.conf").unwrap().name,
            "two words.conf"
        );
        assert!(parse_line("drwxr-xr-x 2 a b 4096 Jan  1 00:00 .").is_none(), "\".\" is not a file");
        // A file called after its own owner used to take the name from the owner
        // column, because that is where the text first appears on the line.
        assert_eq!(parse_line("-rw-r--r-- 1 root root 7 Jan  1 00:00 root").unwrap().name, "root");
        // A symlink's target is not part of its name.
        assert_eq!(
            parse_line("lrwxrwxrwx 1 a b 11 Jan  1 00:00 latest -> /var/log/x").unwrap().name,
            "latest"
        );
        // macOS's sftp-server prints "?" for the link count and pads its columns.
        let real = parse_line(
            "-rw-r--r--    ? you staff       98768 Sep  1 13:30 report.pdf",
        )
        .unwrap();
        assert_eq!(real.name, "report.pdf");
        assert_eq!(real.size, 98768);
        assert!(parse_line("sftp> cd /etc").is_none());
        assert!(parse_line("").is_none());
    }

    /// The far end writes these names, and every caller appends one to the directory
    /// it came from. A name with a slash in it walks out of that directory - and a
    /// failed download then removes what it thinks it created, which for `..` is the
    /// folder above `~/Downloads`.
    #[test]
    fn a_listing_name_is_a_name_and_never_a_path() {
        for hostile in [
            "-rw-r--r-- 1 a b 7 Jan  1 00:00 ../../.ssh/authorized_keys",
            "-rw-r--r-- 1 a b 7 Jan  1 00:00 sub/..",
            "drwxr-xr-x 2 a b 4096 Jan  1 00:00 ..",
        ] {
            assert!(parse_line(hostile).is_none(), "{hostile:?} was taken as a file name");
        }
        assert!(get("nowhere", "/tmp/..", Path::new("/tmp"), true).is_err(), "\"..\" is a folder, not a file");
    }

    /// Walking a tree by appending to the path we sent is how a folder ends up called
    /// `./Desktop/../Downloads/../Downloads`. `pwd` is in the same batch, so it costs
    /// nothing and it is the server's own answer.
    #[test]
    fn a_listing_says_where_it_actually_is() {
        let out = "sftp> cd \"./Desktop/..\"\n\
                   Remote working directory: /Users/me\n\
                   drwxr-xr-x 5 me staff 160 Jan 14 09:31 Downloads\n";
        let l = parse_listing(out, "./Desktop/..");
        assert_eq!(l.path, "/Users/me");
        assert_eq!(l.entries.len(), 1, "the pwd line is not a file");
        assert_eq!(l.entries[0].name, "Downloads");

        // An older server that doesn't echo it leaves us no worse off than before.
        assert_eq!(parse_listing("", "/var/log").path, "/var/log");
    }

    /// sftp stops reading options at the destination, so `-b -` after it is not a
    /// batch flag, it is two stray operands and a usage message.
    #[test]
    fn every_option_lands_before_the_destination() {
        let ssh = ["-J", "bastion", "-p", "2222", "-L", "8080:localhost:80", "-D", "1080", "me@nas"];
        let a = sftp_args(ssh.iter().map(|s| s.to_string()).collect(), Path::new("/tmp/sock"));

        assert_eq!(a.last().unwrap(), "me@nas", "the destination comes last");
        assert!(a.windows(2).any(|w| w == ["-P", "2222"]), "sftp spells the port -P: {a:?}");
        assert!(!a.iter().any(|x| x == "-p" || x == "-L" || x == "8080:localhost:80"));
        assert!(!a.iter().any(|x| x == "-D" || x == "1080"), "every kind of forward goes: {a:?}");
        assert!(a.windows(2).any(|w| w == ["-J", "bastion"]), "the jump chain survives");
        assert!(a.windows(2).any(|w| w == ["-b", "-"]), "the batch flag is here, not appended");
        assert!(a.windows(2).any(|w| w == ["-o", "ServerAliveInterval=15"]), "a dead host is noticed");
    }

    /// sftp's batch language is line-based, so a newline in a path would end the
    /// command and start one of the caller's choosing - and the local half of a
    /// `get`/`put` line is a filename someone else chose just as much as the remote one.
    #[test]
    fn a_path_that_could_smuggle_a_command_is_refused() {
        for bad in ["/tmp/x\nrm -rf /", "/tmp/\"; get /etc/shadow", "/tmp/a\\b", "/tmp/a\rb"] {
            assert!(safe_path(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(safe_path("/var/log/syslog").is_ok());
        assert!(safe_path("/home/me/two words.txt").is_ok());

        // A Windows path is spelled with the character sftp escapes with, so it is
        // rewritten rather than refused - `C:\Users` would otherwise arrive `C:Users`.
        assert_eq!(local_arg(Path::new(r"C:\Users\me\notes.txt")).unwrap(), "C:/Users/me/notes.txt");

        // The local side of the line gets the same treatment.
        let dropped = std::env::temp_dir().join("a\"\nget /etc/shadow");
        assert!(put("nowhere", &dropped, "/tmp").is_err(), "a local name can smuggle too");

        // Both sides of a rename, and the name it would be renamed to.
        assert!(edit("nowhere", "rename", "/tmp/a", "/tmp/b\nrm /etc/passwd").is_err());
        assert!(edit("nowhere", "mkdir", "/tmp/x\"; rm -rf /", "").is_err());
        // And a verb we never meant to offer is not passed along to be interpreted.
        assert!(edit("nowhere", "chmod 777", "/tmp/a", "").is_err());
    }
}
