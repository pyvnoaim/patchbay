//! Lists in and out: an ssh config or a Royal TS document parsed into devices (the
//! window writes what gets ticked), and the device list written back as ssh `Host`
//! blocks. Every key emitted is one `ssh_args` already puts on the command line.

use crate::patchbay::{self, Jack, Jacks};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Imported {
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key: Option<String>,
    pub jump: Option<String>,
    /// Only the Royal TS side fills this; an ssh config has no folders.
    #[serde(default)]
    pub folders: Vec<String>,
    pub rdp: Option<u16>,
    pub url: Option<String>,
    /// `None` is ssh, the default for anything an ssh config named. A Royal TS RDP or
    /// web connection sets `Some(false)`: without it `primary` finds ssh first and
    /// every imported device opens a shell instead of what it was in Royal TS.
    pub ssh: Option<bool>,
    pub os: Option<String>,
    pub desc: Option<String>,
    /// Forwards, spelled the way `patchbay::forward_arg` reads them back.
    pub forward: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Found {
    pub hosts: Vec<Imported>,
    pub warnings: Vec<String>,
}

/// Everything else ssh applies itself when it runs. Forwards are handled separately:
/// a host can have several, and first-wins would drop them.
const WANTED: [&str; 5] = ["hostname", "user", "port", "identityfile", "proxyjump"];

/// `LocalForward 8080 localhost:80` becomes `8080:localhost:80`, the shape ssh also accepts.
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

/// Parser state: the finished hosts, and the `Host` block being read.
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
            // `jump` is one hop; a chain of raw specs can't be synthesised into jacks.
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
                ..Imported::default()
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
        // ssh only treats `#` as a comment at the start of a line; a trailing one is
        // part of the value.
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
        // A Match block's settings hang off conditions, not a host.
        if key == "match" {
            w.flush();
            continue;
        }
        if key == "include" {
            included = true;
            continue;
        }
        // Every forward is kept: a second one is another tunnel, not an override.
        // `-L` is the bare spelling patchbay reads by default.
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
        warnings
            .push("Include lines were not followed - run the importer on those files too".into());
    }

    // A ProxyJump naming another Host points at that jack's sanitised name; anything
    // else is a raw spec passed through to ssh.
    for j in &mut out {
        if let Some(name) = j.jump.as_ref().and_then(|v| by_alias.get(v)) {
            j.jump = Some(name.clone());
        }
    }

    Found {
        hosts: out,
        warnings,
    }
}

/// A word ssh reads as one token in a `Host` line. `Host` takes patterns, so `*`, `?`
/// or `!` would match other hosts, and a newline would start a directive. Refused, not escaped.
fn plain_token(s: &str) -> bool {
    !s.is_empty()
        && !s
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "*?!\"".contains(c))
}

/// A value as ssh reads one: quoted if it has a space, refused if it has a quote or
/// a control character, since escaping is what turns one directive into two.
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
/// names only the next hop: every hop is a block of its own and ssh chains them itself.
fn host_block(name: &str, j: &Jack) -> Option<String> {
    let mut out = format!("Host {name}\n");
    line(&mut out, "HostName", Some(&j.host));
    // Without a HostName the block would only shadow the name.
    if !out.contains("HostName") {
        return None;
    }
    line(&mut out, "User", j.user.as_deref());
    line(&mut out, "Port", j.port.map(|p| p.to_string()).as_deref());
    line(&mut out, "IdentityFile", j.key.as_deref());
    line(&mut out, "ProxyJump", j.jump.as_deref());
    for f in j.forward.iter().flatten() {
        // `forward_arg` is the same check that decides what reaches argv.
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

/// The list as ssh reads it. Names in `theirs` (their own config's `Host` lines) are
/// left out with a reason written into the file, never silently shadowed.
pub fn to_ssh_config(jacks: &Jacks, theirs: &HashSet<String>) -> String {
    let mut out = String::from(
        "# Written by patchbay. Edits here are lost the next time it writes - change\n\
         # the device in the window instead, or turn this off in Settings > Devices.\n",
    );
    let mut skipped: Vec<String> = Vec::new();
    let mut blocks = String::new();

    for (name, j) in jacks {
        if !j.ssh.unwrap_or(true) {
            continue;
        }
        if theirs.contains(name) {
            skipped.push(format!("{name} (your own config already defines it)"));
            continue;
        }
        if !plain_token(name) {
            skipped.push(format!("{name} (not a name ssh can be given)"));
            continue;
        }
        // A jump loop would become a ProxyJump loop ssh only discovers while you wait.
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

/// The `Host` names a config spells out. Exact names only: `Host *` is how people
/// write global options, not a claim on every name. Read off the raw text rather than
/// through `from_ssh_config`, which drops the patterns.
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

// Royal TS. A `.rtsz` is XML despite the z: one flat list of objects under
// `<RTSZDocument>` joined by `ParentID`, so a folder path is a walk up the parents.
// quick-xml rather than by hand because names carry entities (`H&amp;S`).

/// One object in the document.
#[derive(Default)]
struct Node {
    tag: String,
    id: String,
    parent: String,
    fields: HashMap<String, String>,
}

impl Node {
    fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.trim().is_empty())
    }
    fn num(&self, key: &str) -> Option<u16> {
        self.get(key)?.trim().parse().ok()
    }
    fn yes(&self, key: &str) -> bool {
        self.get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }
}

/// Every object in document order. Objects sit at depth 2 under the root, their
/// fields at depth 3.
fn objects(src: &str) -> Result<Vec<Node>, String> {
    use quick_xml::events::Event;
    let mut rd = quick_xml::Reader::from_str(src);
    rd.config_mut().trim_text(true);
    // Importing the first half of a truncated list silently is worse than refusing it.
    rd.config_mut().check_end_names = true;

    let (mut out, mut depth, mut field, mut root) =
        (Vec::<Node>::new(), 0usize, String::new(), String::new());
    loop {
        match rd.read_event() {
            Err(e) => return Err(format!("that document doesn't parse: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match depth {
                    1 => root = name,
                    2 => out.push(Node {
                        tag: name,
                        ..Node::default()
                    }),
                    3 => field = name,
                    _ => {}
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Text(t)) if depth == 3 => {
                if let (Some(node), Ok(v)) = (out.last_mut(), t.unescape()) {
                    node.fields.insert(field.clone(), v.into_owned());
                }
            }
            _ => {}
        }
    }
    // quick-xml is happy to stop inside an element; a truncated file is not an import.
    if depth != 0 {
        return Err("that document stops half way through - is it still copying?".into());
    }
    // Any other XML would otherwise import as an empty document.
    if root != "RTSZDocument" {
        return Err("that isn't a Royal TS document".into());
    }
    for n in out.iter_mut() {
        n.id = n.fields.get("ID").cloned().unwrap_or_default();
        n.parent = n.fields.get("ParentID").cloned().unwrap_or_default();
    }
    Ok(out)
}

/// The folders above a node, outermost first, and whether the walk passed through the
/// trash. Deleted objects are still in the file.
fn place(by_id: &HashMap<&str, &Node>, node: &Node) -> (Vec<String>, bool) {
    let (mut path, mut binned, mut seen) = (Vec::new(), false, HashSet::new());
    let mut at = node.parent.as_str();
    while let Some(p) = by_id.get(at) {
        if !seen.insert(at) {
            break; // a parent loop is a corrupt file, not an infinite one
        }
        match p.tag.as_str() {
            "RoyalFolder" => path.push(p.get("Name").unwrap_or("unnamed").to_string()),
            "RoyalTrash" => binned = true,
            _ => {}
        }
        at = p.parent.as_str();
    }
    path.reverse();
    (path, binned)
}

/// A unique jack name. Royal TS names are only unique among siblings, so a document
/// can hold twenty `DC1`s; the top folder goes in front only where it has to.
fn unique(
    name: &str,
    folders: &[String],
    taken: &mut HashSet<String>,
    clashes: &HashSet<String>,
) -> String {
    let base = match (clashes.contains(name), folders.first()) {
        (true, Some(top)) => format!("{top} {name}"),
        _ => name.to_string(),
    };
    let mut out = base.clone();
    let mut n = 2;
    while !taken.insert(out.clone()) {
        out = format!("{base} {n}");
        n += 1;
    }
    out
}

/// Host to probe and url to open, from an address that may be either. A missing
/// scheme is assumed https.
fn web_parts(uri: &str) -> (String, String) {
    let url = match uri.starts_with("http://") || uri.starts_with("https://") {
        true => uri.to_string(),
        false => format!("https://{uri}"),
    };
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(uri)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(uri)
        .rsplit('@')
        .next()
        .unwrap_or(uri);
    // The port belongs to the url, not the host.
    (host.split(':').next().unwrap_or(host).to_string(), url)
}

/// A Royal TS document parsed into devices. Parses only; the window writes what gets ticked.
pub fn from_royal_ts(src: &str) -> Result<Found, String> {
    let nodes = objects(src)?;
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let clashes: HashSet<String> = {
        let mut seen = HashSet::new();
        nodes
            .iter()
            .filter(|n| n.tag.ends_with("Connection"))
            .filter_map(|n| n.get("Name"))
            .filter(|name| !seen.insert(name.to_string()))
            .map(str::to_string)
            .collect()
    };

    let (mut hosts, mut warnings, mut taken) = (Vec::new(), Vec::new(), HashSet::new());
    let (mut binned, mut serial, mut secrets, mut guessed) = (0usize, 0usize, 0usize, 0usize);
    let mut unknown: HashMap<String, usize> = HashMap::new();

    for n in &nodes {
        if n.get("CredentialPassword").is_some() {
            secrets += 1;
        }
        if !n.tag.ends_with("Connection") {
            continue;
        }
        let (folders, in_trash) = place(&by_id, n);
        if in_trash {
            binned += 1;
            continue;
        }
        let Some(name) = n.get("Name") else { continue };
        let Some(uri) = n.get("URI") else {
            warnings.push(format!("\"{name}\" has no address, so it was left out"));
            continue;
        };
        // patchbay has nothing to open these with.
        if n.yes("IsTelnetConnection") || n.yes("IsSerialPortConnection") {
            serial += 1;
            continue;
        }

        let mut j = Imported {
            name: unique(name, &folders, &mut taken, &clashes),
            user: n.get("CredentialUsername").map(str::to_string),
            desc: n.get("Description").map(str::to_string),
            folders: match folders.is_empty() {
                true => vec![],
                false => vec![folders.join("/")],
            },
            ..Imported::default()
        };
        // A type patchbay can't open is named in the warnings, never guessed at.
        match n.tag.as_str() {
            // Terminal Services and Hyper-V are RDP with a different front end.
            "RoyalRDSConnection" | "RoyalTerminalServicesConnection" | "RoyalHyperVConnection" => {
                j.host = uri.to_string();
                j.rdp = Some(n.num("RDPPort").unwrap_or(3389));
                j.os = Some("windows".into());
                j.ssh = Some(false);
            }
            "RoyalWebConnection" => {
                let (host, url) = web_parts(uri);
                guessed += !uri.starts_with("http") as usize;
                j.host = host;
                j.url = Some(url);
                j.ssh = Some(false);
            }
            "RoyalSSHConnection" => {
                j.host = uri.to_string();
                j.port = n.num("Port").filter(|p| *p != 22);
            }
            other => {
                *unknown.entry(other.to_string()).or_default() += 1;
                continue;
            }
        }
        hosts.push(j);
    }

    for (n, what) in [
        (binned, "in the trash, left there"),
        (serial, "telnet or serial, which patchbay doesn't open"),
        (
            guessed,
            "web addresses with no scheme, so https was assumed",
        ),
    ] {
        if n > 0 {
            warnings.push(format!("{n} {what}"));
        }
    }
    if secrets > 0 {
        warnings.push(format!(
            "{secrets} stored passwords not imported - patchbay keeps none, and yours stay in Royal TS"
        ));
    }
    let mut left = unknown.into_iter().collect::<Vec<_>>();
    left.sort();
    for (tag, n) in left {
        let kind = tag
            .trim_start_matches("Royal")
            .trim_end_matches("Connection");
        warnings.push(format!(
            "{n} {kind} connection(s) skipped - patchbay has no way to open one"
        ));
    }
    Ok(Found { hosts, warnings })
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
                ..Imported::default()
            }
        );

        // Two aliases share the block; a missing HostName means the alias is the host.
        assert_eq!(f.hosts[1].host, "10.0.0.4");
        assert_eq!(f.hosts[2].host, "10.0.0.4");
        assert_eq!(f.hosts[4].host, "bare");

        // `Match` settings must not leak into the block before them.
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
        assert_eq!(
            f.hosts[3].jump.as_deref(),
            Some("10.0.0.4"),
            "the last -J entry"
        );
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
        assert!(
            f.warnings.iter().any(|w| w.contains("Include")),
            "got {:?}",
            f.warnings
        );
    }

    #[test]
    fn a_port_outside_a_u16_or_not_plainly_decimal_is_dropped() {
        let port = |v: &str| from_ssh_config(&format!("Host t\n  Port {v}\n")).hosts[0].port;
        assert_eq!(port("22"), Some(22));
        assert_eq!(
            port("70000"),
            None,
            "would write a config nothing can load back"
        );
        assert_eq!(port("0x16"), None, "Number() reads hex; ssh does not");
        assert_eq!(port("1e3"), None);
        assert_eq!(port("0"), None);
    }

    #[test]
    fn a_jack_comes_out_as_the_host_block_ssh_would_have_wanted() {
        let jacks = patchbay::parse_all(
            "[jack.bastion]\nhost = \"bastion.example\"\nport = 2222\n\n\
             [jack.db]\nhost = \"db.internal\"\nuser = \"deploy\"\njump = \"bastion\"\n\
             key = \"~/.ssh/prod\"\nforward = [\"5432:localhost:5432\", \"-D 1080\"]\n\n\
             [jack.nas]\nhost = \"10.0.0.9\"\nssh = false\nurl = \"https://10.0.0.9\"\n",
        )
        .unwrap()
        .0;
        let out = to_ssh_config(&jacks, &HashSet::new());

        assert!(out.contains("Host db\n"), "{out}");
        assert!(out.contains("    HostName db.internal\n"));
        assert!(out.contains("    User deploy\n"));
        assert!(out.contains("    IdentityFile ~/.ssh/prod\n"));
        assert!(out.contains("    LocalForward 5432:localhost:5432\n"));
        assert!(out.contains("    DynamicForward 1080\n"));
        // The next hop, not the flattened chain.
        assert!(out.contains("    ProxyJump bastion\n"));
        assert!(out.contains("Host bastion\n") && out.contains("    Port 2222\n"));
        assert!(
            !out.contains("Host nas"),
            "a web-only device is not an ssh host: {out}"
        );
    }

    #[test]
    fn a_name_their_own_config_defines_is_left_to_them() {
        let jacks = patchbay::parse_all(
            "[jack.web]\nhost = \"10.0.0.4\"\n[jack.db]\nhost = \"10.0.0.5\"\n",
        )
        .unwrap()
        .0;
        let theirs =
            host_names("Host web\n  HostName elsewhere\nHost *\n  ServerAliveInterval 60\n");
        assert!(theirs.contains("web"));
        let out = to_ssh_config(&jacks, &theirs);
        assert!(!out.contains("Host web\n"), "{out}");
        assert!(out.contains("# left out: web (your own config already defines it)"));
        assert!(out.contains("Host db\n"), "{out}");
    }

    /// A newline starts a directive and a `*` claims other hosts: refused, not escaped.
    #[test]
    fn a_name_or_value_that_could_smuggle_a_directive_is_left_out() {
        for bad in ["ev*il", "two words", "a\nProxyCommand id", "!no", "q\"uote"] {
            assert!(!plain_token(bad), "{bad:?} should not be a Host name");
        }
        assert!(plain_token("prod-web01.eu"));

        assert_eq!(ssh_value("10.0.0.4").as_deref(), Some("10.0.0.4"));
        assert_eq!(
            ssh_value("~/my keys/id").as_deref(),
            Some("\"~/my keys/id\"")
        );
        assert_eq!(ssh_value("x\nProxyCommand id"), None);
        assert_eq!(ssh_value("x\"y"), None);
        assert_eq!(ssh_value("  "), None);

        // End to end: the hostile jack goes, the one beside it stays.
        let jacks = patchbay::parse_all(
            "[jack.ok]\nhost = \"10.0.0.4\"\n[jack.sneaky]\nhost = \"h\\nProxyCommand id\"\n",
        )
        .unwrap()
        .0;
        let out = to_ssh_config(&jacks, &HashSet::new());
        assert!(!out.contains("ProxyCommand"), "{out}");
        assert!(out.contains("Host ok\n"), "{out}");
    }

    #[test]
    fn every_forward_comes_across_in_the_spelling_patchbay_reads_back() {
        let f = from_ssh_config(
            "Host db\n  LocalForward 5432 localhost:5432\n  LocalForward 127.0.0.1:6379 cache:6379\nHost other\n",
        );
        assert_eq!(
            f.hosts[0].forward,
            ["5432:localhost:5432", "127.0.0.1:6379:cache:6379"]
        );
        let g = from_ssh_config(
            "Host tun\n  RemoteForward 9000 localhost:9000\n  DynamicForward 1080\n",
        );
        assert_eq!(g.hosts[0].forward, ["-R 9000:localhost:9000", "-D 1080"]);
        // Forwards must not leak into the next block.
        assert_eq!(f.hosts[1].forward, [] as [String; 0]);
    }

    #[test]
    fn keywords_are_case_insensitive_and_eq_separates_as_well_as_a_space() {
        let f = from_ssh_config("HOST one\n  hostname=10.0.0.7\n  USER  bob\n");
        assert_eq!(f.hosts[0].host, "10.0.0.7");
        assert_eq!(f.hosts[0].user.as_deref(), Some("bob"));
    }
}

#[cfg(test)]
mod royal_tests {
    use super::*;

    /// Shaped like a real document: an entity in a name, and a connection in the trash.
    const DOC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalDocument><ID>doc</ID><Name>Acme</Name></RoyalDocument>
  <RoyalTrash><ID>bin</ID><Name>Trash</Name><ParentID>doc</ParentID></RoyalTrash>
  <RoyalFolder><ID>f1</ID><Name>H&amp;S</Name><ParentID>doc</ParentID></RoyalFolder>
  <RoyalFolder><ID>f2</ID><Name>Server</Name><ParentID>f1</ParentID></RoyalFolder>
  <RoyalFolder><ID>f3</ID><Name>Acme</Name><ParentID>doc</ParentID></RoyalFolder>
  <RoyalRDSConnection><ID>c1</ID><Name>DC1</Name><ParentID>f2</ParentID>
    <URI>10.80.0.50</URI><RDPPort>3389</RDPPort>
    <CredentialUsername>administrator</CredentialUsername>
    <CredentialPassword>xTFOawIB==</CredentialPassword></RoyalRDSConnection>
  <RoyalRDSConnection><ID>c2</ID><Name>DC1</Name><ParentID>f3</ParentID>
    <URI>10.9.0.50</URI><RDPPort>3390</RDPPort></RoyalRDSConnection>
  <RoyalSSHConnection><ID>c3</ID><Name>edge</Name><ParentID>f3</ParentID>
    <URI>10.9.0.1</URI><Port>22</Port>
    <CredentialUsername>ops</CredentialUsername></RoyalSSHConnection>
  <RoyalSSHConnection><ID>c4</ID><Name>console</Name><ParentID>f3</ParentID>
    <URI>10.9.0.2</URI><IsSerialPortConnection>True</IsSerialPortConnection></RoyalSSHConnection>
  <RoyalWebConnection><ID>c5</ID><Name>NAS</Name><ParentID>f3</ParentID>
    <URI>10.9.0.20:5001</URI></RoyalWebConnection>
  <RoyalWebConnection><ID>c6</ID><Name>Proxmox</Name><ParentID>f3</ParentID>
    <URI>https://10.9.0.30:8006/#v1</URI><Description>the hypervisor</Description></RoyalWebConnection>
  <RoyalTerminalServicesConnection><ID>c8</ID><Name>ts</Name><ParentID>f3</ParentID>
    <URI>10.9.0.60</URI></RoyalTerminalServicesConnection>
  <RoyalVncConnection><ID>c9</ID><Name>kvm</Name><ParentID>f3</ParentID>
    <URI>10.9.0.70</URI></RoyalVncConnection>
  <RoyalWebConnection><ID>c7</ID><Name>gone</Name><ParentID>bin</ParentID>
    <URI>10.9.0.99</URI></RoyalWebConnection>
</RTSZDocument>"#;

    fn find<'a>(f: &'a Found, name: &str) -> &'a Imported {
        f.hosts.iter().find(|h| h.name == name).unwrap_or_else(|| {
            panic!(
                "no {name}: {:?}",
                f.hosts.iter().map(|h| &h.name).collect::<Vec<_>>()
            )
        })
    }

    #[test]
    fn a_document_becomes_devices_with_the_way_in_they_already_had() {
        let f = from_royal_ts(DOC).unwrap();
        assert_eq!(
            f.hosts.len(),
            6,
            "{:?}",
            f.hosts.iter().map(|h| &h.name).collect::<Vec<_>>()
        );

        assert_eq!(find(&f, "ts").rdp, Some(3389));
        // Left unset, `primary` picks ssh and the device opens a shell it hasn't got.
        assert_eq!(find(&f, "ts").ssh, Some(false));
        // A type with no answer here is named, not guessed at.
        assert!(
            !f.hosts.iter().any(|h| h.name == "kvm"),
            "a VNC connection was guessed at"
        );
        assert!(
            f.warnings.iter().any(|w| w.contains("Vnc")),
            "{:?}",
            f.warnings
        );

        // The entity has to survive the parent walk.
        let dc = find(&f, "H&S DC1");
        assert_eq!(dc.folders, ["H&S/Server"]);
        assert_eq!(dc.host, "10.80.0.50");
        assert_eq!(dc.rdp, Some(3389));
        assert_eq!(dc.user.as_deref(), Some("administrator"));
        assert_eq!(dc.os.as_deref(), Some("windows"));

        // Two `DC1`s: the customer goes in front of both.
        assert_eq!(find(&f, "Acme DC1").rdp, Some(3390));

        let ssh = find(&f, "edge");
        assert_eq!(
            (ssh.host.as_str(), ssh.port),
            ("10.9.0.1", None),
            "port 22 is not worth writing"
        );
        assert_eq!(ssh.ssh, None, "ssh is the default, not something to write");

        let nas = find(&f, "NAS");
        assert_eq!(nas.ssh, Some(false));
        assert_eq!(nas.url.as_deref(), Some("https://10.9.0.20:5001"));
        assert_eq!(nas.host, "10.9.0.20", "the probe wants a host, not a url");

        let pve = find(&f, "Proxmox");
        assert_eq!(pve.url.as_deref(), Some("https://10.9.0.30:8006/#v1"));
        assert_eq!(pve.host, "10.9.0.30");
        assert_eq!(pve.desc.as_deref(), Some("the hypervisor"));
    }

    #[test]
    fn the_bin_the_serial_port_and_the_passwords_stay_behind() {
        let f = from_royal_ts(DOC).unwrap();
        assert!(
            !f.hosts.iter().any(|h| h.name == "gone"),
            "the trash was imported"
        );
        assert!(
            !f.hosts.iter().any(|h| h.name == "console"),
            "a serial port was imported"
        );

        let said = f.warnings.join(" | ");
        assert!(said.contains("trash"), "{said}");
        assert!(said.contains("serial"), "{said}");
        assert!(said.contains("passwords"), "{said}");
        assert!(said.contains("https was assumed"), "{said}");
    }

    #[test]
    fn a_document_that_is_not_one_says_so_rather_than_importing_nothing() {
        assert!(from_royal_ts("<opml><body/></opml>")
            .unwrap_err()
            .contains("Royal TS"));
        assert!(from_royal_ts("<RTSZDocument><RoyalFolder><ID>f").is_err());
        assert_eq!(
            from_royal_ts("<RTSZDocument></RTSZDocument>")
                .unwrap()
                .hosts
                .len(),
            0
        );
    }
}

#[cfg(test)]
mod royal_smoke {
    use super::*;
    /// Runs only when `ROYAL_TS_DOC=/path/to/doc.rtsz cargo test royal_smoke -- --nocapture`.
    #[test]
    fn a_real_document_reads() {
        let Ok(path) = std::env::var("ROYAL_TS_DOC") else {
            return;
        };
        let src = std::fs::read_to_string(&path).expect("read the document");
        let f = from_royal_ts(&src).expect("parse");
        let (rdp, web, ssh) = (
            f.hosts.iter().filter(|h| h.rdp.is_some()).count(),
            f.hosts.iter().filter(|h| h.url.is_some()).count(),
            f.hosts
                .iter()
                .filter(|h| h.rdp.is_none() && h.url.is_none())
                .count(),
        );
        let folders: std::collections::BTreeSet<_> =
            f.hosts.iter().flat_map(|h| h.folders.clone()).collect();
        println!(
            "\n{} devices: {rdp} rdp, {web} web, {ssh} ssh",
            f.hosts.len()
        );
        println!(
            "{} folders, deepest {}",
            folders.len(),
            folders
                .iter()
                .map(|f| f.matches('/').count() + 1)
                .max()
                .unwrap_or(0)
        );
        println!(
            "qualified names: {}",
            f.hosts.iter().filter(|h| h.name.contains(' ')).count()
        );
        for w in &f.warnings {
            println!("  warning: {w}");
        }
        assert!(
            f.hosts.iter().all(|h| !h.host.trim().is_empty()),
            "a device with no host"
        );
        let names: std::collections::HashSet<_> = f.hosts.iter().map(|h| &h.name).collect();
        assert_eq!(
            names.len(),
            f.hosts.len(),
            "two devices ended up with one name"
        );
    }
}
