//! Host keys the team has seen, kept beside the shared list so "is this the machine
//! we think it is" is answered once per team instead of typed past once per person.
//! ssh does the checking: its argv gains a `GlobalKnownHostsFile`, and that is all.

use crate::patchbay;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Beside the shared list, where everyone on it reads the same one.
pub fn shared() -> Option<PathBuf> {
    patchbay::list().map(|p| p.with_file_name("patchbay_known_hosts"))
}

/// What ssh is actually given: this machine's copy of the shared file, with only the
/// plain lines kept. Beside the config, like the other answers that are this machine's.
fn local() -> PathBuf {
    patchbay::config_path().with_file_name("team_known_hosts")
}

/// A host as `known_hosts` names it: bare on 22, `[host]:port` anywhere else.
pub fn spec(host: &str, port: Option<u16>) -> String {
    match port {
        None | Some(22) => host.to_string(),
        Some(p) => format!("[{host}]:{p}"),
    }
}

/// One `host keytype key` line, or nothing. A `known_hosts` file can say far more than
/// "this host has this key": `@cert-authority` makes ssh trust a signer for whole
/// patterns of hosts, and a `*` matches every one of them. The shared file is somebody
/// else's writing, so only a line naming one exact host and one key is ever believed.
fn plain_line(line: &str) -> Option<String> {
    let f: Vec<&str> = line.split_whitespace().collect();
    let [host, kind, key] = f[..] else {
        return None;
    };
    let named = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c));
    let typed = kind
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-@.".contains(c))
        && ["ssh-", "ecdsa-", "sk-"]
            .iter()
            .any(|p| kind.starts_with(p));
    let keyed = key.len() > 16
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c));
    (named && typed && keyed).then(|| format!("{host} {kind} {key}"))
}

pub fn plain(src: &str) -> String {
    src.lines()
        .filter_map(plain_line)
        .map(|l| l + "\n")
        .collect()
}

/// The shared file with `lines` for `host` added, everything already in it left as
/// written. A key of a kind the team already has for that host, but a different one, is
/// refused: one of the two is not the machine the other saw, and that is for a person
/// to look at, not for a file to settle.
pub fn merged(had: &str, host: &str, lines: &[String]) -> Result<String, String> {
    let mut known = plain(had);
    let mut out = had.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for line in lines {
        let Some(line) = plain_line(line) else {
            continue;
        };
        let kind = line.split(' ').nth(1).unwrap_or_default();
        let same_kind = |l: &&str| {
            let mut f = l.split(' ');
            f.next() == Some(host) && f.next() == Some(kind)
        };
        match known.lines().find(same_kind) {
            Some(theirs) if theirs == line => {}
            Some(_) => {
                return Err(format!(
                    "the team already has another {kind} key for \"{host}\" - one of the two is not the machine the other saw"
                ))
            }
            None => {
                out.push_str(&line);
                out.push('\n');
                known.push_str(&(line + "\n"));
            }
        }
    }
    Ok(out)
}

/// This machine's filtered copy of the shared file, brought up to date. `None` when
/// there is no shared file to copy. Written by rename, so a connect that reads it
/// while another writes it never sees half of one.
fn refreshed() -> Option<PathBuf> {
    let keep = plain(&std::fs::read_to_string(shared()?).ok()?);
    let local = local();
    if std::fs::read_to_string(&local).ok().as_deref() != Some(keep.as_str()) {
        let tmp = local.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, &keep).ok()?;
        std::fs::rename(&tmp, &local).ok()?;
    }
    Some(local)
}

/// Whether two sets of fingerprints (`ED25519 SHA256:…`) name different keys of one
/// kind. A machine that only has an ECDSA key for a host the team recorded as ED25519
/// disagrees with nobody.
fn disagree(mine: &[String], team: &[String]) -> bool {
    let kind = |p: &str| p.split(' ').next().unwrap_or_default().to_string();
    mine.iter().any(|m| {
        let theirs: Vec<&String> = team.iter().filter(|t| kind(t) == kind(m)).collect();
        !theirs.is_empty() && !theirs.contains(&m)
    })
}

/// The option that makes ssh check a team device against what the team has seen. Only
/// for the team's own devices, and nothing at all when there is no shared file: yours
/// are checked against your own `known_hosts`, as they always were.
///
/// **Never when this machine already has another key for the host.** ssh accepts a
/// host if any file it was given has the key on offer, so the team's file would
/// quietly outvote your own: someone able to write the share could put a key there and
/// have you connect to it with no warning. Left out, ssh sees only your key and says
/// what it always said about one that changed.
/// ponytail: the destination only. ssh applies command-line options to the last hop,
/// so a bastion in `-J` is still asked about the old way; a `Host` block per hop if
/// that turns out to matter.
pub fn opts(name: &str, jacks: &patchbay::Jacks) -> Vec<String> {
    if name.starts_with(patchbay::MINE) {
        return vec![];
    }
    let Some(j) = jacks.get(name) else {
        return vec![];
    };
    let k = known(&spec(&j.host, j.port));
    if k.differs {
        return vec![];
    }
    let Some(local) = refreshed() else {
        return vec![];
    };
    let path = local.display().to_string();
    // The value is a list split on spaces, so the path is quoted, and one that holds a
    // quote of its own can't be said.
    if path.contains('"') {
        return vec![];
    }
    // ssh's own defaults first: naming a file here replaces them.
    let defaults = if cfg!(windows) {
        ""
    } else {
        "/etc/ssh/ssh_known_hosts /etc/ssh/ssh_known_hosts2 "
    };
    vec![
        "-o".into(),
        format!("GlobalKnownHostsFile={defaults}\"{path}\""),
    ]
}

fn own_file() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

/// What `ssh-keygen -F` finds for a host in a file, one line each, without its `#`
/// commentary. `-l` asks for fingerprints instead of the lines themselves.
fn found(host: &str, file: &std::path::Path, fingerprints: bool) -> Vec<String> {
    let mut cmd = Command::new("ssh-keygen");
    if fingerprints {
        cmd.arg("-l");
    }
    cmd.arg("-F").arg(host).arg("-f").arg(file);
    let Ok(out) = cmd.stdin(Stdio::null()).stderr(Stdio::null()).output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The fingerprints of a host's keys, as `ED25519 SHA256:…`.
fn prints(host: &str, file: &std::path::Path) -> Vec<String> {
    found(host, file, true)
        .iter()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let at = f.iter().position(|x| x.starts_with("SHA256:"))?;
            Some(format!("{} {}", f.get(at.wrapping_sub(1))?, f[at]))
        })
        .collect()
}

#[derive(serde::Serialize)]
pub struct Known {
    /// What this machine's own `known_hosts` has for it.
    mine: Vec<String>,
    /// What ssh is given of the team's: the filtered copy, not the file as written, so
    /// the window never shows a key ssh would not actually check against.
    team: Vec<String>,
    /// The two name different keys of one kind.
    differs: bool,
}

pub fn known(host: &str) -> Known {
    let mine = own_file().map(|f| prints(host, &f)).unwrap_or_default();
    let team = refreshed().map(|f| prints(host, &f)).unwrap_or_default();
    Known {
        differs: disagree(&mine, &team),
        mine,
        team,
    }
}

/// Put what this machine knows about a host into the team's file. Only ever into a
/// folder that is there: a missing one is a share that is away.
pub fn share(host: &str) -> Result<usize, String> {
    let file = shared().ok_or("there is no shared list to keep host keys beside")?;
    if !file.parent().is_some_and(std::path::Path::is_dir) {
        return Err(format!("{}: that folder is not there", file.display()));
    }
    let own = own_file().ok_or("no ~/.ssh on this machine")?;
    // As stored, the first field may be hashed; the key is what is taken, under the
    // name. A marked line is not a key this machine trusts for the host: `@revoked` is
    // the opposite, and `@cert-authority` is a signer.
    let lines: Vec<String> = found(host, &own, false)
        .iter()
        .filter(|l| !l.trim_start().starts_with('@'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let at = f
                .iter()
                .position(|x| ["ssh-", "ecdsa-", "sk-"].iter().any(|p| x.starts_with(p)))?;
            Some(format!("{host} {} {}", f[at], f.get(at + 1)?))
        })
        .collect();
    if lines.is_empty() {
        return Err(format!(
            "this machine has no key for \"{host}\" yet - connect to it once first"
        ));
    }
    // A file that is there and won't read is not an empty one: writing over it would
    // take everyone's keys with it.
    let read = || match std::fs::read_to_string(&file) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    };
    let had = read()?;
    let next = merged(&had, host, &lines)?;
    let tmp = file.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, &next).map_err(|e| format!("{}: {e}", tmp.display()))?;
    // The same rule the list's writers keep: nothing over a file that moved since.
    if read()? != had {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "{} changed while you were sharing - nothing was written, try again",
            file.display()
        ));
    }
    std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(lines.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIBmockmockmockmockmockmockmockmockmockmockmo";

    #[test]
    fn only_a_line_that_names_one_host_and_one_key_is_believed() {
        let src = format!(
            "# a comment\n\
             10.0.0.4 ssh-ed25519 {KEY}\n\
             [10.0.0.2]:2222 ssh-ed25519 {KEY}\n\
             @cert-authority * ssh-ed25519 {KEY}\n\
             * ssh-ed25519 {KEY}\n\
             10.0.0.5,10.0.0.6 ssh-ed25519 {KEY}\n\
             |1|abc=|def= ssh-ed25519 {KEY}\n\
             10.0.0.7 ssh-ed25519 {KEY} trailing comment\n\
             10.0.0.8 not-a-key {KEY}\n"
        );
        assert_eq!(
            plain(&src),
            format!("10.0.0.4 ssh-ed25519 {KEY}\n[10.0.0.2]:2222 ssh-ed25519 {KEY}\n")
        );
    }

    #[test]
    fn a_second_key_of_the_same_kind_for_a_host_is_refused() {
        // What is already there stays as written, comment and all.
        let had = format!("# ours\n10.0.0.4 ssh-ed25519 {KEY}\n");
        let same = [format!("10.0.0.4 ssh-ed25519 {KEY}")];
        assert_eq!(merged(&had, "10.0.0.4", &same).unwrap(), had);

        let other = [format!("10.0.0.4 ssh-ed25519 {KEY}x")];
        let err = merged(&had, "10.0.0.4", &other).unwrap_err();
        assert!(err.contains("already has another ssh-ed25519 key"), "{err}");

        // Another kind, or another host, is simply added.
        let more = [
            format!("10.0.0.4 ssh-rsa {KEY}"),
            format!("10.0.0.4 @cert-authority {KEY}"),
        ];
        let out = merged(&had, "10.0.0.4", &more).unwrap();
        assert_eq!(out.lines().count(), 3, "{out}");
        assert!(out.starts_with("# ours\n"), "{out}");
    }

    #[test]
    fn keys_only_disagree_within_one_kind() {
        let p = |s: &str| s.to_string();
        let team = [p("ED25519 SHA256:aaa")];
        assert!(!disagree(&[p("ED25519 SHA256:aaa")], &team));
        assert!(disagree(&[p("ED25519 SHA256:bbb")], &team));
        // Another kind of key for the same host is not another machine.
        assert!(!disagree(&[p("ECDSA SHA256:ccc")], &team));
        assert!(!disagree(&[], &team));
    }

    #[test]
    fn a_host_off_22_is_named_the_way_known_hosts_names_it() {
        assert_eq!(spec("10.0.0.4", None), "10.0.0.4");
        assert_eq!(spec("10.0.0.4", Some(22)), "10.0.0.4");
        assert_eq!(spec("10.0.0.2", Some(2222)), "[10.0.0.2]:2222");
    }
}
