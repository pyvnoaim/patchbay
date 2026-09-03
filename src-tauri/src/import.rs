//! The ssh config format, both directions.
//!
//! In: turning an existing ssh config into jacks. Parses only - the window shows the
//! list and writes what you tick through `save_jack`, like any other edit.
//!
//! Out: `to_ssh_config` writes the list back as `Host` blocks, so `ssh web-01` in any
//! terminal reaches what patchbay's Connect reaches, and so do `scp`, `rsync`, Ansible
//! and anything else that reads that file. Every key it emits is one `ssh_args` already
//! puts on the command line - this adds no way to reach a device that patchbay didn't
//! already have.

use crate::patchbay::{self, Jack, Jacks};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, PartialEq, Serialize)]
pub struct Imported {
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key: Option<String>,
    pub jump: Option<String>,
    /// `LocalForward`, `RemoteForward` and `DynamicForward`, spelled the way
    /// `patchbay::forward_arg` reads them back. A tunnel someone set up once is part of
    /// how they reach that host, so importing without it imports half a jack.
    pub forward: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Found {
    pub hosts: Vec<Imported>,
    pub warnings: Vec<String>,
}

/// Everything else ssh already applies for us - we exec it, so importing its defaults
/// would only duplicate them into a second file that can go stale. `localforward` is
/// handled apart from these: a host can have several, and first-wins would drop them.
const WANTED: [&str; 5] = ["hostname", "user", "port", "identityfile", "proxyjump"];

/// `LocalForward 8080 localhost:80` is `8080:localhost:80`; ssh accepts the whole
/// thing written with colons too, which is already the shape we want. The same join
/// does for the other two - `DynamicForward 1080` is one token either way.
fn colon_joined(v: &str) -> String {
    v.split_whitespace().collect::<Vec<_>>().join(":")
}

fn unquote(v: &str) -> &str {
    match v.len() > 1 && v.starts_with('"') && v.ends_with('"') {
        true => &v[1..v.len() - 1],
        false => v,
    }
}

/// A pattern, not a host: ssh matches these, patchbay can't list them.
fn is_pattern(h: &str) -> bool {
    h.starts_with('!') || h.contains('*') || h.contains('?')
}

/// The locals the TypeScript's `flush` closes over. Same fields, same names.
#[derive(Default)]
struct Walk {
    out: Vec<Imported>,
    warnings: Vec<String>,
    by_alias: HashMap<String, String>,
    taken: HashSet<String>,
    aliases: Vec<String>,
    block: HashMap<String, String>,
    forwards: Vec<String>,
}

impl Walk {
    fn flush(&mut self) {
        let mut jump = match self.block.get("proxyjump").map(String::as_str) {
            None | Some("none") => None,
            Some(v) => Some(v.to_string()),
        };
        if let Some(j) = jump.clone().filter(|j| j.contains(',')) {
            // `jump` is the one hop before the target; a chain is built by pointing
            // jacks at each other, which we can't synthesise from a list of raw specs.
            let hops: Vec<&str> = j.split(',').map(str::trim).collect();
            let last = hops.last().unwrap().to_string();
            self.warnings.push(format!(
                "{}: ProxyJump has {} hops - kept \"{last}\", dropped the rest",
                self.aliases.join(", "),
                hops.len()
            ));
            jump = Some(last);
        }
        let forwards = std::mem::take(&mut self.forwards);
        for alias in std::mem::take(&mut self.aliases) {
            // A dot is refused in a jack name, and two aliases can flatten onto one.
            let base = alias.replace('.', "-");
            let mut name = base.clone();
            let mut n = 2;
            while self.taken.contains(&name) {
                name = format!("{base}-{n}");
                n += 1;
            }
            self.taken.insert(name.clone());
            self.by_alias.insert(alias.clone(), name.clone());

            self.out.push(Imported {
                name,
                host: self.block.get("hostname").cloned().unwrap_or(alias),
                user: self.block.get("user").cloned(),
                port: self
                    .block
                    .get("port")
                    .and_then(|p| p.parse().ok())
                    .filter(|p| *p > 0),
                key: self.block.get("identityfile").cloned(),
                jump: jump.clone(),
                forward: forwards.clone(),
            });
        }
        self.block.clear();
    }
}

pub fn from_ssh_config(src: &str) -> Found {
    let mut w = Walk::default();
    let mut included = false;

    for raw in src.lines() {
        let line = raw.trim();
        // ssh only treats `#` as a comment at the start of a line - a trailing one is
        // part of the value, so stripping it would corrupt a path with a hash in it.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let flattened = line.replacen('=', " ", 1);
        let mut parts = flattened.split_whitespace();
        let Some(word) = parts.next() else { continue };
        let key = word.to_lowercase();
        let rest: Vec<&str> = parts.collect();
        let value = unquote(rest.join(" ").trim()).to_string();

        if key == "host" {
            w.flush();
            w.aliases = rest
                .iter()
                .filter(|h| !is_pattern(h))
                .map(|h| h.to_string())
                .collect();
            continue;
        }
        // A Match block's settings hang off conditions, not a host, so nothing in it
        // belongs to a jack.
        if key == "match" {
            w.flush();
            continue;
        }
        if key == "include" {
            included = true;
            continue;
        }
        // Every one of them, in the order ssh would apply them - unlike the rest, a
        // second forward is another tunnel rather than an override. `-L` is the bare
        // spelling patchbay reads by default, so only the other two carry their flag.
        if let Some(flag) = match key.as_str() {
            "localforward" => Some(""),
            "remoteforward" => Some("-R "),
            "dynamicforward" => Some("-D "),
            _ => None,
        } {
            if !value.is_empty() {
                w.forwards.push(format!("{flag}{}", colon_joined(&value)));
            }
            continue;
        }
        // First one wins, the way ssh reads them.
        if WANTED.contains(&key.as_str()) && !value.is_empty() && !w.block.contains_key(&key) {
            w.block.insert(key, value);
        }
    }
    w.flush();

    let Walk {
        mut out,
        mut warnings,
        by_alias,
        ..
    } = w;

    if included {
        warnings.push("Include lines were not followed - run the importer on those files too".into());
    }

    // A ProxyJump naming another Host has to point at that jack's sanitised name;
    // anything else is a raw spec, which patchbay passes through to ssh untouched.
    for j in &mut out {
        if let Some(name) = j.jump.as_ref().and_then(|v| by_alias.get(v)) {
            j.jump = Some(name.clone());
        }
    }

    Found { hosts: out, warnings }
}

/// A word ssh will read as one token in a `Host` line. `Host` takes *patterns*, so a
/// name carrying `*`, `?` or `!` would match hosts it was never meant to, and a newline
/// would start a directive of someone else's choosing - the same hole a `.rdp` has, and
/// refused the same way rather than escaped.
fn plain_token(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control() || c.is_whitespace() || "*?!\"".contains(c))
}

/// A value as ssh reads one. A space is legal inside double quotes, because a key
/// really does live in a path with a space in it often enough; a quote or a control
/// character is refused, since escaping is what turns one directive into two.
fn ssh_value(v: &str) -> Option<String> {
    let v = v.trim();
    if v.is_empty() || v.chars().any(|c| c.is_control() || c == '"') {
        return None;
    }
    Some(match v.contains(' ') {
        true => format!("\"{v}\""),
        false => v.to_string(),
    })
}

fn line(out: &mut String, key: &str, v: Option<&str>) {
    if let Some(v) = v.and_then(ssh_value) {
        out.push_str(&format!("    {key} {v}\n"));
    }
}

/// One jack as a `Host` block, or None if it has nothing to say to ssh. `ProxyJump`
/// carries only the *immediate* hop, never the flattened chain: every hop is a `Host`
/// block of its own here, and ssh chains ProxyJump itself. A jump that isn't a jack is
/// a raw `user@host`, which is what ProxyJump wants anyway.
fn host_block(name: &str, j: &Jack) -> Option<String> {
    let mut out = format!("Host {name}\n");
    line(&mut out, "HostName", Some(&j.host));
    // A block with no HostName is a block that does nothing but shadow the name.
    if !out.contains("HostName") {
        return None;
    }
    line(&mut out, "User", j.user.as_deref());
    line(&mut out, "Port", j.port.map(|p| p.to_string()).as_deref());
    line(&mut out, "IdentityFile", j.key.as_deref());
    line(&mut out, "ProxyJump", j.jump.as_deref());
    for f in j.forward.iter().flatten() {
        // Refused rather than emitted broken: `forward_arg` is the same check that
        // decides what reaches argv, so the file can never say more than a connect would.
        if let Ok((flag, spec)) = patchbay::forward_arg(f) {
            let key = match flag {
                "-R" => "RemoteForward",
                "-D" => "DynamicForward",
                _ => "LocalForward",
            };
            line(&mut out, key, Some(spec));
        }
    }
    Some(out)
}

/// The list as ssh reads it. `theirs` is the `Host` names their own config already
/// defines, and those are left out: they wrote that file, and quietly shadowing a host
/// someone has used for years is the one way this could do real damage.
///
/// Left out with a reason written into the file, not silently - it is the only place
/// anyone would go looking when `ssh web` doesn't reach what the window reaches.
pub fn to_ssh_config(jacks: &Jacks, theirs: &HashSet<String>) -> String {
    let mut out = String::from(
        "# Written by patchbay. Edits here are lost the next time it writes - change\n\
         # the device in the window instead, or turn this off in Settings > Devices.\n",
    );
    let mut skipped: Vec<String> = Vec::new();
    let mut blocks = String::new();

    for (name, j) in jacks {
        if !j.ssh.unwrap_or(true) {
            continue;   // a web-only or RDP-only device has nothing to say to ssh
        }
        if theirs.contains(name) {
            skipped.push(format!("{name} (your own config already defines it)"));
            continue;
        }
        if !plain_token(name) {
            skipped.push(format!("{name} (not a name ssh can be given)"));
            continue;
        }
        // Reuses the cycle guard rather than repeating it: a jump loop would become a
        // ProxyJump loop, and ssh would only find out about it while you waited.
        if patchbay::hops(name, jacks).is_err() {
            skipped.push(format!("{name} (its jump chain doesn't resolve)"));
            continue;
        }
        if let Some(b) = host_block(name, j) {
            blocks.push('\n');
            blocks.push_str(&b);
        }
    }

    for s in &skipped {
        out.push_str(&format!("# left out: {s}\n"));
    }
    out.push_str(&blocks);
    out
}

/// The `Host` names a config already spells out. Exact names only, deliberately: a
/// pattern is how people write global options, and `Host *` claiming every name would
/// leave nothing to write. A pattern that overlaps one of ours still applies for every
/// keyword we don't set, which is what someone writing `Host prod-*` meant anyway.
///
/// Read off the raw text rather than through `from_ssh_config`, which drops the
/// patterns - and a name it dropped is still a name we must not shadow.
pub fn host_names(src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for l in src.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("Host ").or_else(|| l.strip_prefix("host ")) else {
            continue;
        };
        out.extend(rest.split_whitespace().map(str::to_string));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
# my hosts
Host *
  ServerAliveInterval 30

Host bastion.example
  HostName 203.0.113.9
  User ops
  Port 2222
  IdentityFile ~/.ssh/ops

Host web prod-web
  HostName 10.0.0.4
  ProxyJump bastion.example

Host db
  HostName db.internal
  ProxyJump bastion.example,10.0.0.4

Host bare

Match host *.internal
  User root
"#;

    #[test]
    fn hosts_become_jacks_patterns_and_match_blocks_do_not() {
        let f = from_ssh_config(CONFIG);
        let names: Vec<&str> = f.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["bastion-example", "web", "prod-web", "db", "bare"]);

        assert_eq!(
            f.hosts[0],
            Imported {
                name: "bastion-example".into(),
                host: "203.0.113.9".into(),
                user: Some("ops".into()),
                port: Some(2222),
                key: Some("~/.ssh/ops".into()),
                jump: None,
                forward: vec![],
            }
        );

        // Two aliases on one Host line share the block, and a missing HostName means
        // the alias was already the hostname.
        assert_eq!(f.hosts[1].host, "10.0.0.4");
        assert_eq!(f.hosts[2].host, "10.0.0.4");
        assert_eq!(f.hosts[4].host, "bare");

        // `Match` settings hang off a condition, not a host - nothing there is a jack,
        // and it must not leak into the block before it.
        assert_eq!(f.hosts[4].user, None);
    }

    #[test]
    fn a_jump_pointing_at_another_host_follows_it_to_the_renamed_jack() {
        let f = from_ssh_config(CONFIG);
        assert_eq!(f.hosts[1].jump.as_deref(), Some("bastion-example"));
    }

    #[test]
    fn a_multi_hop_proxyjump_keeps_the_hop_nearest_the_target_and_says_so() {
        let f = from_ssh_config(CONFIG);
        assert_eq!(f.hosts[3].jump.as_deref(), Some("10.0.0.4"), "the last -J entry");
        assert!(
            f.warnings.iter().any(|w| w.contains("dropped the rest")),
            "got {:?}",
            f.warnings
        );
    }

    #[test]
    fn colliding_names_get_a_suffix_rather_than_overwriting_each_other() {
        let f = from_ssh_config("Host a.b\nHost a-b\n");
        let names: Vec<&str> = f.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["a-b", "a-b-2"]);
    }

    #[test]
    fn an_unfollowed_include_is_reported_not_silently_skipped() {
        let f = from_ssh_config("Include ~/.ssh/work/*\nHost x\n");
        assert!(f.warnings.iter().any(|w| w.contains("Include")), "got {:?}", f.warnings);
    }

    #[test]
    fn a_port_outside_a_u16_or_not_plainly_decimal_is_dropped() {
        let port = |v: &str| from_ssh_config(&format!("Host t\n  Port {v}\n")).hosts[0].port;
        assert_eq!(port("22"), Some(22));
        assert_eq!(port("70000"), None, "would write a config nothing can load back");
        assert_eq!(port("0x16"), None, "Number() reads hex; ssh does not");
        assert_eq!(port("1e3"), None);
        assert_eq!(port("0"), None);
    }

    /// The file makes `ssh db` do what Connect does, so the keys have to be the ones
    /// `ssh_args` builds - and `ProxyJump` names the next hop only, because the hop is a
    /// `Host` block here too and ssh chains them itself.
    #[test]
    fn a_jack_comes_out_as_the_host_block_ssh_would_have_wanted() {
        let jacks = patchbay::parse(
            "[jack.bastion]\nhost = \"bastion.example\"\nport = 2222\n\n\
             [jack.db]\nhost = \"db.internal\"\nuser = \"deploy\"\njump = \"bastion\"\n\
             key = \"~/.ssh/prod\"\nforward = [\"5432:localhost:5432\", \"-D 1080\"]\n\n\
             [jack.nas]\nhost = \"10.0.0.9\"\nssh = false\nurl = \"https://10.0.0.9\"\n",
        )
        .unwrap();
        let out = to_ssh_config(&jacks, &HashSet::new());

        assert!(out.contains("Host db\n"), "{out}");
        assert!(out.contains("    HostName db.internal\n"));
        assert!(out.contains("    User deploy\n"));
        assert!(out.contains("    IdentityFile ~/.ssh/prod\n"));
        assert!(out.contains("    LocalForward 5432:localhost:5432\n"));
        assert!(out.contains("    DynamicForward 1080\n"));
        // The next hop, not the flattened chain - ssh walks the rest itself.
        assert!(out.contains("    ProxyJump bastion\n"));
        assert!(out.contains("Host bastion\n") && out.contains("    Port 2222\n"));
        // A device ssh can't reach has nothing to say here.
        assert!(!out.contains("Host nas"), "a web-only device is not an ssh host: {out}");
    }

    /// Their file is theirs. A name it already spells out is left alone and said so in
    /// the only place anyone would look when `ssh web` doesn't go where the window does.
    #[test]
    fn a_name_their_own_config_defines_is_left_to_them() {
        let jacks = patchbay::parse("[jack.web]\nhost = \"10.0.0.4\"\n[jack.db]\nhost = \"10.0.0.5\"\n").unwrap();
        let theirs = host_names("Host web\n  HostName elsewhere\nHost *\n  ServerAliveInterval 60\n");
        assert!(theirs.contains("web"));
        let out = to_ssh_config(&jacks, &theirs);
        assert!(!out.contains("Host web\n"), "{out}");
        assert!(out.contains("# left out: web (your own config already defines it)"));
        // `Host *` is how people write global options, not a claim on every name.
        assert!(out.contains("Host db\n"), "{out}");
    }

    /// ssh_config is line-based and `Host` takes patterns, so both are the `.rdp` hole
    /// in another spelling: a newline starts a directive, a `*` claims hosts it wasn't
    /// given. Refused rather than escaped, and never at the cost of the rest of the file.
    #[test]
    fn a_name_or_value_that_could_smuggle_a_directive_is_left_out() {
        for bad in ["ev*il", "two words", "a\nProxyCommand id", "!no", "q\"uote"] {
            assert!(!plain_token(bad), "{bad:?} should not be a Host name");
        }
        assert!(plain_token("prod-web01.eu"));

        assert_eq!(ssh_value("10.0.0.4").as_deref(), Some("10.0.0.4"));
        assert_eq!(ssh_value("~/my keys/id").as_deref(), Some("\"~/my keys/id\""));
        assert_eq!(ssh_value("x\nProxyCommand id"), None);
        assert_eq!(ssh_value("x\"y"), None);
        assert_eq!(ssh_value("  "), None);

        // And end to end: the hostile jack goes, the one beside it stays.
        let jacks = patchbay::parse(
            "[jack.ok]\nhost = \"10.0.0.4\"\n[jack.sneaky]\nhost = \"h\\nProxyCommand id\"\n",
        )
        .unwrap();
        let out = to_ssh_config(&jacks, &HashSet::new());
        assert!(!out.contains("ProxyCommand"), "{out}");
        assert!(out.contains("Host ok\n"), "{out}");
    }

    #[test]
    fn every_forward_comes_across_in_the_spelling_patchbay_reads_back() {
        let f = from_ssh_config(
            "Host db\n  LocalForward 5432 localhost:5432\n  LocalForward 127.0.0.1:6379 cache:6379\nHost other\n",
        );
        assert_eq!(f.hosts[0].forward, ["5432:localhost:5432", "127.0.0.1:6379:cache:6379"]);
        // The other two carry the flag patchbay reads them back by.
        let g = from_ssh_config(
            "Host tun\n  RemoteForward 9000 localhost:9000\n  DynamicForward 1080\n",
        );
        assert_eq!(g.hosts[0].forward, ["-R 9000:localhost:9000", "-D 1080"]);
        // A block's forwards belong to that block and must not leak into the next.
        assert_eq!(f.hosts[1].forward, [] as [String; 0]);
    }

    #[test]
    fn keywords_are_case_insensitive_and_eq_separates_as_well_as_a_space() {
        let f = from_ssh_config("HOST one\n  hostname=10.0.0.7\n  USER  bob\n");
        assert_eq!(f.hosts[0].host, "10.0.0.7");
        assert_eq!(f.hosts[0].user.as_deref(), Some("bob"));
    }
}
