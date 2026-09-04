//! The config as data: load, `[defaults]` inheritance, the jump-chain walk, name
//! resolve, and every ssh argv the app ever builds. Pure logic with its tests below;
//! `commands/` is the surface over it.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub type Jacks = IndexMap<String, Jack>;

/// Everything optional so `[defaults]` and `[jack.x]` deserialize with one shape;
/// `host` is checked when a jack is used, not when it is parsed.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Jack {
    #[serde(default)]
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key: Option<String>,
    pub jump: Option<String>,
    pub os: Option<String>,
    /// Optional web UI; a NAS or router is one device with two ways in.
    pub url: Option<String>,
    /// Port for remote desktop. Absent means this device has none.
    pub rdp: Option<u16>,
    /// Port for screen sharing, handed to the system's VNC viewer.
    pub vnc: Option<u16>,
    /// Absent means yes.
    pub ssh: Option<bool>,
    /// What Enter opens: "ssh" | "rdp" | "vnc" | "web". Absent picks the first one the
    /// device has.
    pub primary: Option<String>,
    pub folders: Option<Vec<String>>,
    /// ponytail: the old name for `folders`, read but never written; drop it once nobody
    /// has one.
    pub tags: Option<Vec<String>>,
    pub desc: Option<String>,
    pub forward: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    defaults: Jack,
    #[serde(default)]
    jack: IndexMap<String, Jack>,
    #[serde(default)]
    folder: IndexMap<String, Folder>,
}

/// What is said *about* a folder. A folder exists because a device names it, so a note
/// on an empty one does not make the tree grow a row.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Folder {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `$PATCHBAY_CONFIG` if set; else `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` or
/// `~/.config` elsewhere.
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("PATCHBAY_CONFIG") {
        return PathBuf::from(p);
    }
    let home = if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(x)
    } else if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".config"))
    } else {
        dirs::home_dir().unwrap_or_default().join(".config")
    };
    home.join("patchbay").join("patchbay.toml")
}

/// Where session logs land: beside the config, because they are this machine's and not
/// part of the list.
pub fn logs_dir() -> PathBuf {
    config_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("logs")
}

fn expand(p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) => format!("{}{}", dirs::home_dir().unwrap_or_default().display(), rest),
        None => p.to_string(),
    }
}

/// The notes hung on folders, by full path. Read separately from `Jacks` because a
/// folder is not a device and every caller wants one or the other.
pub fn notes(src: &str) -> IndexMap<String, String> {
    let raw: Raw = match toml::from_str(src) {
        Ok(r) => r,
        Err(_) => return IndexMap::new(),
    };
    raw.folder
        .into_iter()
        .filter_map(|(k, f)| Some((k, f.note?)))
        .filter(|(_, n)| !n.trim().is_empty())
        .collect()
}

/// `[defaults]` merges into every jack; the jack's own value wins.
pub fn parse(src: &str) -> Result<Jacks, String> {
    let raw: Raw = toml::from_str(src).map_err(|e| e.message().to_string())?;
    let d = &raw.defaults;
    Ok(raw
        .jack
        .into_iter()
        .map(|(name, j)| {
            let merged = Jack {
                host: if j.host.is_empty() {
                    d.host.clone()
                } else {
                    j.host
                },
                user: j.user.or_else(|| d.user.clone()),
                port: j.port.or(d.port),
                key: j.key.or_else(|| d.key.clone()),
                jump: j.jump.or_else(|| d.jump.clone()),
                os: j.os.or_else(|| d.os.clone()),
                url: j.url.or_else(|| d.url.clone()),
                rdp: j.rdp.or(d.rdp),
                vnc: j.vnc.or(d.vnc),
                ssh: j.ssh.or(d.ssh),
                primary: j.primary.or_else(|| d.primary.clone()),
                folders: j
                    .folders
                    .or(j.tags)
                    .or_else(|| d.folders.clone().or_else(|| d.tags.clone())),
                tags: None,
                desc: j.desc.or_else(|| d.desc.clone()),
                forward: j.forward.or_else(|| d.forward.clone()),
            };
            (name, merged)
        })
        .collect())
}

/// A missing file is an empty list (a first run), not an error. A file that won't parse
/// still is one.
pub fn load(path: &Path) -> Result<Jacks, String> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Jacks::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    parse(&src).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn spec(j: &Jack) -> String {
    match &j.user {
        Some(u) => format!("{u}@{}", j.host),
        None => j.host.clone(),
    }
}

/// A destination on its way into argv. A leading `-` would be an ssh option, and
/// `task_argv` appends an operand after the chain, which is what a smuggled
/// `-oProxyCommand=` needs to run. This is where "nothing in a config is executed" holds.
fn dest(s: String) -> Result<String, String> {
    if s.trim().is_empty() {
        return Err("that jack has no host to connect to".into());
    }
    if s.starts_with('-') {
        return Err(format!(
            "\"{s}\" isn't a usable host - ssh would read it as an option"
        ));
    }
    Ok(s)
}

/// The same rule for a jack; `host` is checked before `spec` hides an empty one behind
/// `user@`.
fn dest_of(j: &Jack) -> Result<String, String> {
    dest(j.host.clone())?;
    dest(spec(j))
}

/// The jump chain as `ssh -J` wants it, first hop from here leftmost. Walking `jump`
/// goes outward from the target, so the walk is reversed.
pub fn hops(name: &str, jacks: &Jacks) -> Result<Vec<String>, String> {
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([name.to_string()]);
    let mut hop = j.jump.clone();
    while let Some(h) = hop {
        if !seen.insert(h.clone()) {
            return Err(format!("jump loop through \"{h}\""));
        }
        match jacks.get(&h) {
            // Not a jack name: a raw ssh spec, passed through.
            None => {
                out.push(dest(h)?);
                break;
            }
            Some(via) => {
                out.push(match via.port {
                    Some(p) => format!("{}:{p}", dest_of(via)?),
                    None => dest_of(via)?,
                });
                hop = via.jump.clone();
            }
        }
    }
    out.reverse();
    Ok(out)
}

/// The machine the first TCP connection goes to: the outermost bastion, or the jack
/// itself. The only thing worth probing; anything past it is reachable only through ssh.
pub fn entry(name: &str, jacks: &Jacks) -> Result<(String, u16), String> {
    let mut cur = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    let mut seen: HashSet<String> = HashSet::from([name.to_string()]);
    while let Some(h) = cur.jump.clone() {
        if !seen.insert(h.clone()) {
            return Err(format!("jump loop through \"{h}\""));
        }
        match jacks.get(&h) {
            Some(via) => cur = via,
            // A raw spec: user@host, host:port, or both.
            None => {
                let hp = h.rsplit('@').next().unwrap_or(&h);
                return Ok(
                    match hp
                        .rsplit_once(':')
                        .and_then(|(a, b)| b.parse().ok().map(|p| (a, p)))
                    {
                        Some((host, port)) => (host.to_string(), port),
                        None => (hp.to_string(), 22),
                    },
                );
            }
        }
    }
    Ok((cur.host.clone(), probe_port(cur)))
}

/// Which port the status dot tests. A device with `ssh = false` has nothing on 22.
fn probe_port(j: &Jack) -> u16 {
    if j.ssh.unwrap_or(true) {
        return j.port.unwrap_or(22);
    }
    if let Some(p) = j.rdp.or(j.vnc) {
        return p;
    }
    j.url.as_deref().and_then(url_port).unwrap_or(443)
}

/// The port a `url` points at: explicit if it has one, else the scheme's default.
fn url_port(url: &str) -> Option<u16> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    // An IPv6 literal is bracketed, so only a colon past the bracket can be a port.
    let after = host.rsplit_once(']').map_or(host, |(_, a)| a);
    if let Some((_, p)) = after.rsplit_once(':') {
        if let Ok(p) = p.parse() {
            return Some(p);
        }
    }
    Some(if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        443
    })
}

pub fn ssh_args(name: &str, jacks: &Jacks) -> Result<Vec<String>, String> {
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    let hop_list = hops(name, jacks)?;

    let mut args: Vec<String> = Vec::new();
    if !hop_list.is_empty() {
        args.push("-J".into());
        args.push(hop_list.join(","));
    }
    if let Some(p) = j.port {
        args.push("-p".into());
        args.push(p.to_string());
    }
    if let Some(k) = &j.key {
        args.push("-i".into());
        args.push(expand(k));
    }
    for f in j.forward.iter().flatten() {
        let (flag, spec) = forward_arg(f)?;
        args.push(flag.into());
        args.push(spec.into());
    }
    args.push(dest_of(j)?);
    Ok(args)
}

/// Every flag `forward_arg` can hand back. Anything that strips forwards goes through
/// this, so it drops the flag and its operand for every kind.
pub const FORWARD_FLAGS: [&str; 3] = ["-L", "-R", "-D"];

/// Which ssh flag a forward is, and the spec after it. A bare spec is `-L`; `-R` and
/// `-D` are written as ssh's own flag. The flag is matched exactly and the rest may not
/// start with `-`, because this becomes argv and a config must not supply an option.
pub fn forward_arg(spec: &str) -> Result<(&'static str, &str), String> {
    let spec = spec.trim();
    let (flag, rest) = match spec.split_once(' ') {
        Some(("-L", r)) => ("-L", r),
        Some(("-R", r)) => ("-R", r),
        Some(("-D", r)) => ("-D", r),
        _ => ("-L", spec),
    };
    let rest = rest.trim();
    if rest.is_empty() || rest.starts_with('-') {
        return Err(format!(
            "\"{spec}\" isn't a forward - write \"8080:localhost:80\", or -R or -D and its own"
        ));
    }
    Ok((flag, rest))
}

/// The local port a forward binds, which a standing tunnel watches. A `-R` has none.
/// ponytail: a bracketed IPv6 bind address gives None rather than the wrong port.
pub fn forward_local(spec: &str) -> Option<u16> {
    let (flag, rest) = forward_arg(spec).ok()?;
    let parts: Vec<&str> = rest.split(':').collect();
    match (flag, parts.len()) {
        // -D is `[bind:]port`: the whole forward is the local end.
        ("-D", 1) => parts[0].parse().ok(),
        ("-D", 2) => parts[1].parse().ok(),
        ("-R", _) | ("-D", _) => None,
        (_, 3) => parts[0].parse().ok(),
        (_, 4) => parts[1].parse().ok(),
        _ => None,
    }
}

/// Drop every forward, flag and operand. A one-shot check or an sftp run has no use
/// for them, and re-binding a port a live session holds only prints an error.
pub fn without_forwards(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if FORWARD_FLAGS.contains(&a.as_str()) {
            it.next();
        } else {
            out.push(a);
        }
    }
    out
}

/// Ping or traceroute toward a device, as `(program, args)`. A device behind a jump
/// is not reachable from here, so the check runs *on the hop*, over the same chain.
pub fn task_argv(task: &str, name: &str, jacks: &Jacks) -> Result<(String, Vec<String>), String> {
    if task != "ping" && task != "trace" {
        return Err(format!("no task named \"{task}\""));
    }
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    let host = plain_host(&j.host)?;

    let Some(hop) = &j.jump else {
        return Ok(local_task(task, host));
    };
    // ponytail: the far side is assumed POSIX. It answers ssh, so it isn't cmd.exe.
    let mut args = match jacks.contains_key(hop) {
        true => without_forwards(ssh_args(hop, jacks)?),
        // A jump that isn't a jack is a raw ssh spec with no chain of its own.
        false => vec![hop.clone()],
    };
    args.extend(posix_task(task, host));
    Ok(("ssh".into(), args))
}

/// The far-side form hands the host to the hop's shell, so a space or a `;` would run
/// there as a command. Refused rather than quoted, like a newline in a `.rdp`.
fn plain_host(host: &str) -> Result<&str, String> {
    let ok = !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".:-_".contains(c));
    ok.then_some(host)
        .ok_or_else(|| format!("\"{host}\" isn't a plain host name to check"))
}

/// `ping -c 5` / `traceroute`: the spelling the far side has, and the local one
/// everywhere but Windows.
fn posix_task(task: &str, host: &str) -> Vec<String> {
    match task {
        "trace" => vec!["traceroute".into(), host.into()],
        _ => vec!["ping".into(), "-c".into(), "5".into(), host.into()],
    }
}

#[cfg(not(target_os = "windows"))]
fn local_task(task: &str, host: &str) -> (String, Vec<String>) {
    let mut v = posix_task(task, host);
    (v.remove(0), v)
}

#[cfg(target_os = "windows")]
fn local_task(task: &str, host: &str) -> (String, Vec<String>) {
    match task {
        "trace" => ("tracert".into(), vec![host.into()]),
        _ => ("ping".into(), vec!["-n".into(), "5".into(), host.into()]),
    }
}

/// Only http(s) may reach the desktop opener or a webview: `open` and `explorer` will
/// happily launch an application for any other scheme.
pub fn is_web_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && !u.contains(['\n', '\r', '\0'])
}

/// Exact name wins; otherwise substring match, but only if it's unambiguous.
pub fn resolve(query: &str, jacks: &Jacks) -> Result<String, String> {
    if jacks.contains_key(query) {
        return Ok(query.to_string());
    }
    let hits: Vec<&String> = jacks.keys().filter(|n| n.contains(query)).collect();
    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(format!("no jack matching \"{query}\"")),
        n => Err(format!(
            "\"{query}\" matches {n} jacks: {}",
            hits.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `user@host:2222` typed into the palette, as an argv. No defaults, jump or key are
/// looked up; the same `dest` guard applies because this is argv all the same.
pub fn adhoc_args(query: &str) -> Result<Vec<String>, String> {
    let q = query.trim();
    let (target, port) = match q.rsplit_once(':') {
        Some((t, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (t, Some(p)),
        _ => (q, None),
    };
    if target.is_empty() || q.chars().any(char::is_whitespace) {
        return Err(format!("no jack matching \"{q}\""));
    }
    let target = dest(target.to_string())?;
    let mut args = Vec::new();
    if let Some(p) = port {
        args.push("-p".into());
        args.push(p.into());
    }
    args.push(target);
    Ok(args)
}

/// How a device is reached: "ssh", "rdp", "vnc" or "web". `primary` is still read for
/// configs that set several, and ignored when it names something the device lost.
pub fn primary(j: &Jack) -> String {
    // "sftp" is ssh with a different default action.
    let has = |k: &str| match k {
        "ssh" | "sftp" => j.ssh.unwrap_or(true),
        "rdp" => j.rdp.is_some(),
        "vnc" => j.vnc.is_some(),
        _ => j.url.is_some(),
    };
    j.primary
        .as_deref()
        .filter(|p| has(p))
        .map(str::to_string)
        .unwrap_or_else(|| {
            ["ssh", "rdp", "vnc", "web"]
                .iter()
                .find(|k| has(k))
                .unwrap_or(&"ssh")
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Jacks {
        parse(
            r#"
            [jack.bastion]
            host = "bastion.example"
            user = "jump"
            port = 2222

            [jack.web]
            host = "10.0.0.4"
            user = "deploy"
            key  = "~/.ssh/prod"
            jump = "bastion"

            [jack.db]
            host = "10.0.0.5"
            jump = "web"
            forward = ["5432:localhost:5432"]

            [jack.loop]
            host = "a"
            jump = "loop2"

            [jack.loop2]
            host = "b"
            jump = "loop"

            [jack.raw]
            host = "c"
            jump = "someone@elsewhere"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn a_quick_connect_is_the_query_and_nothing_else() {
        assert_eq!(adhoc_args("root@10.0.0.5").unwrap(), vec!["root@10.0.0.5"]);
        assert_eq!(adhoc_args("box:2222").unwrap(), vec!["-p", "2222", "box"]);
        assert!(adhoc_args("-oProxyCommand=id").is_err());
        assert!(adhoc_args("two words").is_err());
        assert!(adhoc_args(":22").is_err());
    }

    #[test]
    fn a_missing_config_is_a_first_run_not_an_error() {
        let p = std::env::temp_dir().join(format!("patchbay-{}-missing.toml", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert!(load(&p).unwrap().is_empty());
        std::fs::write(&p, "[jack.x]\nhost = 1\n").unwrap();
        assert!(load(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }

    /// `site/index.html` carries a JS mirror of `hops` and `ssh_args` for its demo
    /// panel; the two must be updated together.
    #[test]
    fn the_config_on_the_website_still_resolves_to_the_argv_it_shows() {
        let j = parse(
            r#"
[defaults]
user = "root"

[jack.bastion]
host = "bastion.example"
port = 2222

[jack.web]
host = "10.0.0.4"
user = "deploy"
jump = "bastion"

[jack.db]
host = "10.0.0.5"
jump = "web"
forward = ["5432:localhost:5432"]
"#,
        )
        .unwrap();

        assert_eq!(
            ssh_args("db", &j).unwrap().join(" "),
            "-J root@bastion.example:2222,deploy@10.0.0.4 -L 5432:localhost:5432 root@10.0.0.5",
            "site/index.html shows this argv for this config; update both together"
        );
    }

    #[test]
    fn plain_jack_is_just_user_at_host() {
        assert_eq!(
            ssh_args("bastion", &fixture()).unwrap(),
            ["-p", "2222", "jump@bastion.example"]
        );
    }

    #[test]
    fn jump_chains_dial_the_outermost_bastion_first() {
        // db via web via bastion: from here the order is bastion, then web.
        assert_eq!(
            ssh_args("db", &fixture()).unwrap(),
            [
                "-J",
                "jump@bastion.example:2222,deploy@10.0.0.4",
                "-L",
                "5432:localhost:5432",
                "10.0.0.5"
            ]
        );
        assert_eq!(
            hops("web", &fixture()).unwrap(),
            ["jump@bastion.example:2222"]
        );
    }

    #[test]
    fn entry_is_the_outermost_hop_not_the_target() {
        let j = fixture();
        assert_eq!(entry("db", &j).unwrap(), ("bastion.example".into(), 2222));
        assert_eq!(
            entry("bastion", &j).unwrap(),
            ("bastion.example".into(), 2222)
        );
        assert_eq!(entry("raw", &j).unwrap(), ("elsewhere".into(), 22));
    }

    #[test]
    fn a_device_without_ssh_is_probed_where_it_actually_listens() {
        let j = parse(
            r#"
            [jack.dc]
            host = "192.168.1.26"
            ssh = false
            rdp = 3389

            [jack.nas]
            host = "10.0.0.20"
            ssh = false
            url = "https://10.0.0.20:5001"

            [jack.gateway]
            host = "10.0.0.1"
            ssh = false
            url = "https://10.0.0.1"

            [jack.mac]
            host = "10.0.0.30"
            ssh = false
            vnc = 5900

            [jack.both]
            host = "10.0.0.9"
            rdp = 3389
            "#,
        )
        .unwrap();
        assert_eq!(entry("dc", &j).unwrap().1, 3389);
        assert_eq!(
            entry("mac", &j).unwrap().1,
            5900,
            "screen sharing is where it listens"
        );
        assert_eq!(entry("nas", &j).unwrap().1, 5001);
        assert_eq!(entry("gateway", &j).unwrap().1, 443, "https with no port");
        assert_eq!(
            entry("both", &j).unwrap().1,
            22,
            "ssh is still the way in when it has it"
        );
    }

    #[test]
    fn tilde_expands_and_unknown_jump_passes_through_raw() {
        let j = fixture();
        let joined = ssh_args("web", &j).unwrap().join(" ");
        assert!(
            joined.contains("-i /"),
            "key should expand to an absolute path: {joined}"
        );
        assert!(
            joined.ends_with("/.ssh/prod deploy@10.0.0.4"),
            "got {joined}"
        );
        assert_eq!(
            ssh_args("raw", &j).unwrap(),
            ["-J", "someone@elsewhere", "c"]
        );
    }

    #[test]
    fn jump_loops_error_instead_of_hanging() {
        assert!(ssh_args("loop", &fixture()).unwrap_err().contains("loop"));
    }

    #[test]
    fn resolve_exact_wins_unique_substring_works_ambiguity_errors() {
        let j = fixture();
        assert_eq!(resolve("web", &j).unwrap(), "web");
        assert_eq!(resolve("bast", &j).unwrap(), "bastion");
        assert_eq!(resolve("loop", &j).unwrap(), "loop"); // exact beats the loop2 substring hit
        assert!(resolve("loo", &j).unwrap_err().contains("matches 2"));
        assert!(resolve("nope", &j).unwrap_err().contains("no jack"));
    }

    #[test]
    fn defaults_merge_into_jacks_jack_wins() {
        let j = parse(
            r#"
            [defaults]
            user = "root"

            [jack.a]
            host = "h1"

            [jack.b]
            host = "h2"
            user = "me"
            "#,
        )
        .unwrap();
        assert_eq!(j["a"].user.as_deref(), Some("root"));
        assert_eq!(j["b"].user.as_deref(), Some("me"));
    }

    #[test]
    fn tags_still_reads_as_folders_and_folders_wins_when_both_are_there() {
        let j = parse(
            r#"
            [jack.old]
            host = "h1"
            tags = ["prod/eu"]

            [jack.new]
            host = "h2"
            folders = ["prod/us"]
            tags = ["stale"]
            "#,
        )
        .unwrap();
        assert_eq!(
            j["old"].folders.as_deref(),
            Some(&["prod/eu".to_string()][..])
        );
        assert_eq!(
            j["new"].folders.as_deref(),
            Some(&["prod/us".to_string()][..])
        );
        // Normalised away on load, so nothing downstream has to know the old name.
        assert!(j["old"].tags.is_none() && j["new"].tags.is_none());
    }

    #[test]
    fn a_jacks_own_tags_beats_folders_inherited_from_defaults() {
        let j = parse(
            r#"
            [defaults]
            folders = ["inherited"]

            [jack.a]
            host = "h1"
            tags = ["mine"]
            "#,
        )
        .unwrap();
        assert_eq!(j["a"].folders.as_deref(), Some(&["mine".to_string()][..]));
    }

    #[test]
    fn jacks_keep_file_order() {
        let j = fixture();
        assert_eq!(
            j.keys().take(3).map(|s| s.as_str()).collect::<Vec<_>>(),
            ["bastion", "web", "db"]
        );
    }

    #[test]
    fn a_forwards_local_port_is_the_one_before_the_target() {
        assert_eq!(forward_local("5432:localhost:5432"), Some(5432));
        assert_eq!(forward_local("127.0.0.1:8080:10.0.0.9:80"), Some(8080));
        assert_eq!(forward_local("0.0.0.0:8080:10.0.0.9:80"), Some(8080));
        // Not a port to watch: a socket path, a bracketed v6 bind, nonsense.
        assert_eq!(forward_local("8080:/run/thing.sock"), None);
        assert_eq!(forward_local("[::1]:8080:h:80"), None);
        assert_eq!(forward_local("70000:h:80"), None);
        // A SOCKS proxy binds one port here; a remote forward binds none.
        assert_eq!(forward_local("-D 1080"), Some(1080));
        assert_eq!(forward_local("-D 127.0.0.1:1080"), Some(1080));
        assert_eq!(forward_local("-R 9000:localhost:9000"), None);
    }

    /// The value reaches argv, so the flag is matched exactly or a config could hand ssh
    /// an option of its own.
    #[test]
    fn a_forward_is_one_of_three_flags_and_never_an_option_of_its_own() {
        assert_eq!(
            forward_arg("8080:localhost:80").unwrap(),
            ("-L", "8080:localhost:80")
        );
        assert_eq!(
            forward_arg("-R 9000:localhost:9000").unwrap(),
            ("-R", "9000:localhost:9000")
        );
        assert_eq!(forward_arg("-D 1080").unwrap(), ("-D", "1080"));
        for bad in ["-o ProxyCommand=id", "-L", "-L8080:h:80", "--", "-D", "-R "] {
            assert!(forward_arg(bad).is_err(), "{bad:?} should be refused");
        }
    }

    /// A config may produce an ssh argv and nothing else: a host beginning with `-` is
    /// an option, and `task_argv` would supply the operand it needs to run.
    #[test]
    fn a_host_can_never_become_an_ssh_option() {
        let j = parse(
            r#"
            [jack.evil]
            host = "-oProxyCommand=touch /tmp/pwned"

            [jack.behind]
            host = "10.0.0.4"
            jump = "evil"

            [jack.raw]
            host = "10.0.0.5"
            jump = "-oProxyCommand=touch /tmp/pwned"

            [jack.nohost]
            user = "root"
            "#,
        )
        .unwrap();
        for name in ["evil", "behind", "raw", "nohost"] {
            assert!(ssh_args(name, &j).is_err(), "{name:?} produced an argv");
        }
        // The hop is what `task_argv` builds on, so the chain has to refuse it too.
        assert!(hops("behind", &j).is_err());
        assert!(hops("raw", &j).is_err());
        // A `user@` in front must not make it safe, or safety would depend on `[defaults]`.
        let dressed = parse("[jack.x]\nhost = \"-oProxyCommand=id\"\nuser = \"root\"\n").unwrap();
        assert!(ssh_args("x", &dressed).is_err());
        let fine = parse("[jack.x]\nhost = \"10.0.0.4\"\nuser = \"root\"\n").unwrap();
        assert_eq!(
            ssh_args("x", &fine).unwrap().last().unwrap(),
            "root@10.0.0.4"
        );
    }

    #[test]
    fn every_kind_of_forward_reaches_argv_behind_its_own_flag() {
        let j = parse(
            "[jack.x]\nhost = \"h\"\nforward = [\"8080:localhost:80\", \"-R 9000:localhost:9000\", \"-D 1080\"]\n",
        )
        .unwrap();
        let a = ssh_args("x", &j).unwrap();
        assert!(
            a.windows(2).any(|w| w == ["-L", "8080:localhost:80"]),
            "got {a:?}"
        );
        assert!(
            a.windows(2).any(|w| w == ["-R", "9000:localhost:9000"]),
            "got {a:?}"
        );
        assert!(a.windows(2).any(|w| w == ["-D", "1080"]), "got {a:?}");

        // A broken one fails the connection rather than passing through.
        let bad = parse("[jack.x]\nhost = \"h\"\nforward = [\"-o ProxyCommand=id\"]\n").unwrap();
        assert!(ssh_args("x", &bad).is_err());
    }

    const CHAIN: &str = r#"
[jack.bastion]
host = "bastion.example"
user = "ops"
port = 2222
forward = ["9000:localhost:9000"]

[jack.db]
host = "db.internal"
jump = "bastion"

[jack.plain]
host = "10.0.0.4"

[jack.raw]
host = "10.0.0.9"
jump = "ops@edge.example"

[jack.sneaky]
host = "x; id"
"#;

    #[test]
    fn a_check_runs_here_when_it_can_and_on_the_hop_when_it_cannot() {
        let j = parse(CHAIN).unwrap();

        let (p, a) = task_argv("ping", "plain", &j).unwrap();
        assert_eq!(p, "ping");
        assert!(a.contains(&"10.0.0.4".to_string()), "got {a:?}");

        // Behind a bastion it runs there, and the hop's own tunnel is left out or it
        // fights the live session for the port.
        let (p, a) = task_argv("ping", "db", &j).unwrap();
        assert_eq!(p, "ssh");
        assert_eq!(
            a,
            [
                "-p",
                "2222",
                "ops@bastion.example",
                "ping",
                "-c",
                "5",
                "db.internal"
            ]
        );

        let (_, a) = task_argv("trace", "db", &j).unwrap();
        assert_eq!(a.last().unwrap(), "db.internal");
        assert!(a.contains(&"traceroute".to_string()), "got {a:?}");

        let (_, a) = task_argv("ping", "raw", &j).unwrap();
        assert_eq!(a[0], "ops@edge.example");
    }

    #[test]
    fn a_host_that_could_be_a_command_on_the_hop_is_refused() {
        let j = parse(CHAIN).unwrap();
        assert!(
            task_argv("ping", "sneaky", &j).is_err(),
            "a space reaches the hop's shell"
        );
        assert!(
            task_argv("nope", "plain", &j).is_err(),
            "only the two tasks exist"
        );
    }

    #[test]
    fn only_http_and_https_are_openable() {
        assert!(is_web_url("https://10.0.0.20:5001"));
        assert!(is_web_url("HTTP://nas.local/"));
        assert!(is_web_url("  https://nas.local  "));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "x-apple-helpme://boom",
            "smb://share",
            "nas.local:5001",
            "",
            "https://ok\nfile:///etc/passwd",
        ] {
            assert!(!is_web_url(bad), "{bad:?} should be rejected");
        }
    }
}
