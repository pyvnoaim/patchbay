//! Turning an existing ssh config into jacks. Parses only: the window shows the list
//! and writes what you tick through `save_jack`, like any other edit.

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
    /// `LocalForward`, spelled the way `-L` wants it. A tunnel someone set up once is
    /// part of how they reach that host, so importing without it imports half a jack.
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

/// `LocalForward 8080 localhost:80` is `-L 8080:localhost:80`; ssh accepts the whole
/// thing written with colons too, which is already the shape we want.
fn as_dash_l(v: &str) -> String {
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
        // second LocalForward is another tunnel rather than an override.
        if key == "localforward" && !value.is_empty() {
            w.forwards.push(as_dash_l(&value));
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

    #[test]
    fn every_local_forward_comes_across_in_the_spelling_dash_l_wants() {
        let f = from_ssh_config(
            "Host db\n  LocalForward 5432 localhost:5432\n  LocalForward 127.0.0.1:6379 cache:6379\nHost other\n",
        );
        assert_eq!(f.hosts[0].forward, ["5432:localhost:5432", "127.0.0.1:6379:cache:6379"]);
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
