//! Port of `src/patchbay.ts`. Same behaviour, same errors, same argv.
//! The tests at the bottom mirror `test/patchbay.test.ts` one for one — if you
//! change the TypeScript, change this, and both suites should still agree.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

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
    /// Optional web UI — a NAS or router is one device with two ways in.
    pub url: Option<String>,
    /// Port for remote desktop. Absent means this device has none.
    pub rdp: Option<u16>,
    /// Absent means yes — most devices are reached over ssh.
    pub ssh: Option<bool>,
    /// What Enter and a double-click do: "ssh" | "rdp" | "web". Absent picks the
    /// first one the device actually has.
    pub primary: Option<String>,
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
}

/// `%APPDATA%` on Windows, `$XDG_CONFIG_HOME` or `~/.config` elsewhere — matches configPath() in the CLI.
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

/// `[defaults]` merges into every jack — that's the whole credential-inheritance feature.
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
                ssh: j.ssh.or(d.ssh),
                primary: j.primary.or_else(|| d.primary.clone()),
                tags: j.tags.or_else(|| d.tags.clone()),
                desc: j.desc.or_else(|| d.desc.clone()),
                forward: j.forward.or_else(|| d.forward.clone()),
            };
            (name, merged)
        })
        .collect())
}

pub fn load(path: &std::path::Path) -> Result<Jacks, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&src).map_err(|e| format!("{}: {e}", path.display()))
}

fn spec(j: &Jack) -> String {
    match &j.user {
        Some(u) => format!("{u}@{}", j.host),
        None => j.host.clone(),
    }
}

/// The jump chain, ordered the way `ssh -J` wants it: leftmost is the first hop
/// from here. Walking `jump` goes outward from the target, so the walk is reversed —
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

/// The machine we open the first TCP connection to — the outermost bastion if
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
    Ok((cur.host.clone(), cur.port.unwrap_or(22)))
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
        // db is reached via web, web via bastion — so from here the order is bastion, then web.
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
    fn jacks_keep_file_order() {
        let j = fixture();
        assert_eq!(j.keys().take(3).map(|s| s.as_str()).collect::<Vec<_>>(), ["bastion", "web", "db"]);
    }
}
