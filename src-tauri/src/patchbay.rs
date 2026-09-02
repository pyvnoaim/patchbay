//! Port of `src/patchbay.ts`. Same behaviour, same errors, same argv.
//! The tests at the bottom mirror `test/patchbay.test.ts` one for one - if you
//! change the TypeScript, change this, and both suites should still agree.

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
    /// Optional web UI - a NAS or router is one device with two ways in.
    pub url: Option<String>,
    /// Port for remote desktop. Absent means this device has none.
    pub rdp: Option<u16>,
    /// Port for screen sharing. Handed to the system's VNC viewer, never spoken here.
    pub vnc: Option<u16>,
    /// Absent means yes - most devices are reached over ssh.
    pub ssh: Option<bool>,
    /// What Enter and a double-click do: "ssh" | "rdp" | "vnc" | "web". Absent picks the
    /// first one the device actually has.
    pub primary: Option<String>,
    pub folders: Option<Vec<String>>,
    /// ponytail: the old name for `folders`. Read so existing files still work,
    /// never written; drop it once nobody has one.
    pub tags: Option<Vec<String>>,
    pub desc: Option<String>,
    pub forward: Option<Vec<String>>,
    /// Which space this came from - the file it was in, not a field anyone writes.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    defaults: Jack,
    #[serde(default)]
    jack: IndexMap<String, Jack>,
}

/// `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` or `~/.config` elsewhere - matches configPath() in the CLI.
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

fn expand(p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) => format!("{}{}", dirs::home_dir().unwrap_or_default().display(), rest),
        None => p.to_string(),
    }
}

/// `[defaults]` merges into every jack - that's the whole credential-inheritance feature.
pub fn parse(src: &str) -> Result<Jacks, String> {
    let raw: Raw = toml::from_str(src).map_err(|e| e.message().to_string())?;
    let d = &raw.defaults;
    Ok(raw
        .jack
        .into_iter()
        .map(|(name, j)| {
            let merged = Jack {
                host: if j.host.is_empty() { d.host.clone() } else { j.host },
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
                space: None,
            };
            (name, merged)
        })
        .collect())
}

pub fn load(path: &Path) -> Result<Jacks, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&src).map_err(|e| format!("{}: {e}", path.display()))
}

/// Beside the config: one file per extra space. A space *is* a config, whole.
pub fn spaces_dir(cfg: &Path) -> PathBuf {
    cfg.with_file_name("spaces")
}

/// Where a named space lives. `None` is the main config - that one is your own list.
pub fn space_path(cfg: &Path, space: Option<&str>) -> PathBuf {
    match space {
        Some(s) => spaces_dir(cfg).join(format!("{s}.toml")),
        None => cfg.to_path_buf(),
    }
}

/// Every space that exists: the main config first, then `spaces/*.toml` sorted.
/// The `.toml` test is load-bearing - a space's `.toml.base` and `.toml.bak` sit in
/// the same directory and are not spaces.
pub fn space_paths(cfg: &Path) -> Vec<(Option<String>, PathBuf)> {
    let mut out = Vec::new();
    if cfg.exists() {
        out.push((None, cfg.to_path_buf()));
    }
    let Ok(dir) = std::fs::read_dir(spaces_dir(cfg)) else {
        return out;
    };
    let mut spaces: Vec<(Option<String>, PathBuf)> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| Some((Some(p.file_stem()?.to_str()?.to_string()), p)))
        .collect();
    // read_dir is unordered, and which of two colliding names wins must not depend
    // on the filesystem.
    spaces.sort();
    out.append(&mut spaces);
    out
}

/// Every space's jacks in one map. Each file resolves on its own, so `[defaults]` in
/// a space applies to that space's jacks and nobody else's.
///
/// ponytail: a name in two spaces resolves to the first one - the main config, then
/// spaces alphabetically. Qualify as "acme:web" if two spaces ever collide in practice.
pub fn load_all(cfg: &Path) -> Result<Jacks, String> {
    let mut out = Jacks::new();
    for (space, path) in space_paths(cfg) {
        for (name, mut j) in load(&path)? {
            if !out.contains_key(&name) {
                j.space = space.clone();
                out.insert(name, j);
            }
        }
    }
    Ok(out)
}

fn spec(j: &Jack) -> String {
    match &j.user {
        Some(u) => format!("{u}@{}", j.host),
        None => j.host.clone(),
    }
}

/// The jump chain, ordered the way `ssh -J` wants it: leftmost is the first hop
/// from here. Walking `jump` goes outward from the target, so the walk is reversed -
/// `db → web → bastion` has to dial bastion first, not web.
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
            // not a jack name, pass through as a raw ssh spec
            None => {
                out.push(h);
                break;
            }
            Some(via) => {
                out.push(match via.port {
                    Some(p) => format!("{}:{p}", spec(via)),
                    None => spec(via),
                });
                hop = via.jump.clone();
            }
        }
    }
    out.reverse();
    Ok(out)
}

/// The machine we open the first TCP connection to - the outermost bastion if
/// there's a chain, otherwise the jack itself. This is the only thing worth probing;
/// anything past it is reachable only through ssh.
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
            // raw spec: user@host, host:port, or both
            None => {
                let hp = h.rsplit('@').next().unwrap_or(&h);
                return Ok(match hp.rsplit_once(':').and_then(|(a, b)| b.parse().ok().map(|p| (a, p))) {
                    Some((host, port)) => (host.to_string(), port),
                    None => (hp.to_string(), 22),
                });
            }
        }
    }
    Ok((cur.host.clone(), probe_port(cur)))
}

/// Which port the status dot should test. A device that declares `ssh = false` has
/// nothing listening on 22, so probing it anyway reported every RDP-only and
/// web-only device as down while they were perfectly reachable.
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
    Some(if scheme.eq_ignore_ascii_case("http") { 80 } else { 443 })
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
        args.push("-L".into());
        args.push(f.clone());
    }
    args.push(spec(j));
    Ok(args)
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
            hits.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// How a device is reached: "ssh", "rdp", "vnc" or "web". The sheet's radio, resolved in
/// one place so the CLI refuses what the window wouldn't offer. `primary` is still read
/// for configs that set several, and ignored when it names something the device lost.
pub fn primary(j: &Jack) -> String {
    // "sftp" is ssh with a different default action, so it needs ssh and nothing else.
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

/// `bay ls <filter>` - a name or a folder, substring either way.
#[allow(dead_code)]   // the `bay` bin's; the window filters in JS
pub fn matches(j: &Jack, name: &str, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(f) => name.contains(f) || j.folders.iter().flatten().any(|x| x.contains(f)),
    }
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
    fn plain_jack_is_just_user_at_host() {
        assert_eq!(ssh_args("bastion", &fixture()).unwrap(), ["-p", "2222", "jump@bastion.example"]);
    }

    #[test]
    fn jump_chains_dial_the_outermost_bastion_first() {
        // db is reached via web, web via bastion - so from here the order is bastion, then web.
        assert_eq!(
            ssh_args("db", &fixture()).unwrap(),
            ["-J", "jump@bastion.example:2222,deploy@10.0.0.4", "-L", "5432:localhost:5432", "10.0.0.5"]
        );
        assert_eq!(hops("web", &fixture()).unwrap(), ["jump@bastion.example:2222"]);
    }

    #[test]
    fn entry_is_the_outermost_hop_not_the_target() {
        let j = fixture();
        assert_eq!(entry("db", &j).unwrap(), ("bastion.example".into(), 2222));
        assert_eq!(entry("bastion", &j).unwrap(), ("bastion.example".into(), 2222));
        assert_eq!(entry("raw", &j).unwrap(), ("elsewhere".into(), 22));
    }

    #[test]
    fn a_device_without_ssh_is_probed_where_it_actually_listens() {
        let j = parse(
            r#"
            [jack.dc]
            host = "10.0.0.26"
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
        assert_eq!(entry("mac", &j).unwrap().1, 5900, "screen sharing is where it listens");
        assert_eq!(entry("nas", &j).unwrap().1, 5001);
        assert_eq!(entry("gateway", &j).unwrap().1, 443, "https with no port");
        assert_eq!(entry("both", &j).unwrap().1, 22, "ssh is still the way in when it has it");
    }

    #[test]
    fn tilde_expands_and_unknown_jump_passes_through_raw() {
        let j = fixture();
        let joined = ssh_args("web", &j).unwrap().join(" ");
        assert!(joined.contains("-i /"), "key should expand to an absolute path: {joined}");
        assert!(joined.ends_with("/.ssh/prod deploy@10.0.0.4"), "got {joined}");
        assert_eq!(ssh_args("raw", &j).unwrap(), ["-J", "someone@elsewhere", "c"]);
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
        assert_eq!(j["old"].folders.as_deref(), Some(&["prod/eu".to_string()][..]));
        assert_eq!(j["new"].folders.as_deref(), Some(&["prod/us".to_string()][..]));
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
        // Mirrors the TypeScript test of the same name - the two disagreed here once.
        assert_eq!(j["a"].folders.as_deref(), Some(&["mine".to_string()][..]));
    }

    #[test]
    fn jacks_keep_file_order() {
        let j = fixture();
        assert_eq!(j.keys().take(3).map(|s| s.as_str()).collect::<Vec<_>>(), ["bastion", "web", "db"]);
    }

    #[test]
    fn every_space_loads_the_main_config_wins_a_collision_and_defaults_stay_put() {
        // Mirrors the TypeScript test of the same name.
        let dir = std::env::temp_dir().join(format!("patchbay-{}-spaces", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("spaces")).unwrap();
        let cfg = dir.join("patchbay.toml");
        std::fs::write(&cfg, "[jack.mine]\nhost = \"h1\"\n\n[jack.both]\nhost = \"ours\"\n").unwrap();
        std::fs::write(
            dir.join("spaces/acme.toml"),
            "[defaults]\nuser = \"root\"\n\n[jack.theirs]\nhost = \"h2\"\n\n[jack.both]\nhost = \"theirs\"\n",
        )
        .unwrap();
        // Neither is a space: they sit beside one and end in something else.
        std::fs::write(dir.join("spaces/acme.toml.base"), "[jack.stale]\nhost = \"old\"\n").unwrap();
        std::fs::write(dir.join("spaces/acme.toml.bak"), "[jack.older]\nhost = \"older\"\n").unwrap();

        let all = load_all(&cfg).unwrap();
        let mut names: Vec<&String> = all.keys().collect();
        names.sort();
        assert_eq!(names, ["both", "mine", "theirs"]);
        assert_eq!(all["mine"].space, None);
        assert_eq!(all["theirs"].space.as_deref(), Some("acme"));
        assert_eq!(all["both"].host, "ours");
        // A space's [defaults] are that space's, not everyone's.
        assert_eq!(all["theirs"].user.as_deref(), Some("root"));
        assert_eq!(all["mine"].user, None);
    }

    #[test]
    fn no_spaces_directory_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-nospaces", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("patchbay.toml");
        std::fs::write(&cfg, "[jack.a]\nhost = \"h1\"\n").unwrap();
        assert_eq!(load_all(&cfg).unwrap().keys().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn primary_names_how_a_device_is_reached_and_falls_back_when_it_points_at_nothing() {
        let how = |extra: &str| {
            let j = parse(&format!("[jack.x]\nhost = \"h\"\n{extra}")).unwrap();
            primary(&j["x"])
        };
        assert_eq!(how(""), "ssh");
        assert_eq!(how("ssh = false\nurl = \"https://x\""), "web");
        assert_eq!(how("ssh = false\nrdp = 3389"), "rdp");
        assert_eq!(how("ssh = false\nvnc = 5900"), "vnc");
        assert_eq!(how("url = \"https://x\""), "ssh", "ssh wins unless the device says otherwise");
        assert_eq!(how("primary = \"web\"\nurl = \"https://x\""), "web");
        assert_eq!(how("primary = \"rdp\""), "ssh", "a primary pointing at what's gone falls back");
    }
}
