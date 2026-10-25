//! The config as data: load, `[defaults]` inheritance, the jump-chain walk, name
//! resolve, and every ssh argv the app ever builds. Pure logic with its tests below;
//! `commands/` is the surface over it.
//!
//! Jump chains resolve by walking `jump` until it hits a non-jack name (passed through
//! raw) or runs out. The cycle guard is load-bearing; don't drop it.
//!
//! `-J` order is reversed relative to the walk. Walking `jump` goes *outward* from the
//! target, but `ssh -J a,b` dials `a` first. `db → web → bastion` must emit `-J
//! bastion,web`. There's a test pinning it - that bug is invisible until a chain is
//! three deep.
//!
//! A `forward` is `-L` unless it says otherwise - `-R <spec>` and `-D <spec>` are
//! written as ssh's own flag, and `forward_arg` matches those three exactly and refuses
//! anything else starting with `-`. That check is the whole safety story: the value
//! becomes argv, and `-L` used to be safe for free by never being anything but an
//! operand. Everything that *strips* forwards goes through `FORWARD_FLAGS`, or an sftp
//! run keeps the `-D` a session already bound. A `-R` has no local port, so
//! `forward_local` gives `None` and `Tunnels::open` judges that tunnel by ssh still
//! being alive.
//!
//! "Find the break" asks one link at a time (`break_steps` in `patchbay.rs`,
//! `find_break` in `commands/jacks.rs`): a TCP connect to the first hop from here, then
//! each hop is asked over the chain before it whether the next one answers, with
//! `BatchMode=yes` so a hop that wants a password says so instead of waiting. ssh's own
//! exit 255 means the hop we asked *from* would not let us in, so that is where the
//! break is drawn. It only reads, and only runs when asked.
//!
//! The status dot probes the chain's *entry point* (the outermost bastion), not the
//! target. Anything past the first hop is only reachable through ssh, so there's
//! nothing to TCP-probe. A serial console has no address at all and is never probed.
//!
//! Telnet and serial are the system's own tools on the pty, like ssh: `telnet`, or
//! curl's telnet client where there is none (macOS dropped `telnet`), and `picocom`,
//! `screen` or `cu` for a serial port. A telnet device behind a bastion is reached
//! through the same local forward RDP and VNC use, so `telnet_argv` only ever sees an
//! address. Nothing from the file reaches argv unchecked: a host goes through
//! `plain_host`, a serial device has to be a plain `/dev/` path, a baud rate one of the
//! standard ones.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub type Jacks = IndexMap<String, Jack>;

/// What a device in your own vault is called once a shared list is set: `me:web` beside
/// the team's `web`. Only ever in memory, never in a file. A `:` can't be part of a
/// `patchbay://` link name, so a link always means the team's device.
pub const MINE: &str = "me:";

/// Everything optional so `[defaults]` and `[jack.x]` deserialize with one shape;
/// `host` is checked when a jack is used, not when it is parsed.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Jack {
    /// What the window calls it, when that isn't the key: names are unique per folder,
    /// keys per list, so a second `web` in another folder is `[jack.web-2]` with this.
    /// Jumps and links go by the key. Never inherited.
    pub name: Option<String>,
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
    /// Port for screen sharing, in a tab or the system's VNC viewer.
    pub vnc: Option<u16>,
    /// Port for telnet: old switches, and the ports of a console server.
    pub telnet: Option<u16>,
    /// A serial port on this machine, `/dev/cu.usbserial-1410`: a console cable. A
    /// device reached only this way needs no `host`.
    pub serial: Option<String>,
    /// The serial port's speed; 9600 when absent, which is what nearly every console is.
    pub baud: Option<u32>,
    /// Absent means yes.
    pub ssh: Option<bool>,
    /// What Enter opens: "ssh" | "rdp" | "vnc" | "telnet" | "serial" | "web". Absent
    /// picks the first one the device has.
    pub primary: Option<String>,
    pub folders: Option<Vec<String>>,
    pub desc: Option<String>,
    pub forward: Option<Vec<String>>,
    /// Hardware address, for Wake on LAN. Nothing else reads it: patchbay wakes a
    /// device, it does not address one by it.
    pub mac: Option<String>,
    /// Set by `merge` on a device of your own whose `jump` may name a team device while
    /// the team list is away. Never in a file.
    #[serde(skip)]
    pub waits: bool,
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
///
/// The four settings are the ones a whole customer's site shares - their bastion, the
/// account, the key, a non-standard port - and they inherit like `[defaults]` does, so
/// `jump` is written once rather than on every device behind it. Nothing else belongs
/// here: a `host` or a `url` is about one device by definition.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Folder {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
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

/// A shared list, when `[settings].list` in *this machine's* config names one. Read
/// from the own file only, so a shared file can't point `list` somewhere else.
pub fn list_at(own: &Path) -> Option<PathBuf> {
    let l = crate::config::load_settings_at(own).list?;
    let l = l.trim();
    let p = PathBuf::from(expand(l));
    // The own config named as its own shared list would be one file read as two vaults,
    // every device in it a copy of itself - and "Remove the copies" would empty it.
    let same =
        p == own || matches!((p.canonicalize(), own.canonicalize()), (Ok(a), Ok(b)) if a == b);
    (!l.is_empty() && !same).then_some(p)
}

pub fn list() -> Option<PathBuf> {
    list_at(&config_path())
}

/// Where the devices, `[defaults]` and folder notes live: the shared list if there is
/// one, else the own config. `[settings]` and `[colors]` are always in the own config.
pub fn list_path() -> PathBuf {
    list().unwrap_or_else(config_path)
}

/// Where session logs land: beside the config, because they are this machine's and not
/// part of the list.
pub fn logs_dir() -> PathBuf {
    config_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("logs")
}

pub fn expand(p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) => format!("{}{}", dirs::home_dir().unwrap_or_default().display(), rest),
        None => p.to_string(),
    }
}

/// Everything said about folders, by full path: a note, the settings the folder lends
/// its devices, or both. Read separately from `Jacks` because a folder is not a device
/// and every caller wants one or the other.
pub fn folders(src: &str) -> IndexMap<String, Folder> {
    match toml::from_str::<Raw>(src) {
        Ok(raw) => raw.folder,
        Err(_) => IndexMap::new(),
    }
}

/// A snippet is a line of text typed onto a prompt, never a command that is run: the
/// person at the keyboard presses Enter. That promise is this check and nothing else,
/// the way `dest` is for a host. No control character at all: `\r` and `\n` are Enter,
/// `\x03` is ^C, `\x1b` starts an escape sequence, and a tab asks the shell to complete.
pub fn snippet_ok(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("a snippet needs some text".into());
    }
    if text.chars().any(char::is_control) {
        return Err(
            "a snippet is one line of plain text - it can't press Enter or a control key".into(),
        );
    }
    if text.chars().count() > 1000 {
        return Err("a snippet is at most 1000 characters".into());
    }
    Ok(())
}

/// The `[snippet]` table, in file order, and the names of the ones that were refused.
/// Read apart from `Raw` so one bad value costs that snippet and not the whole list:
/// this file may be a colleague's, and it never went through a save here.
pub fn snippets(src: &str) -> (Vec<(String, String)>, Vec<String>) {
    let table = toml::from_str::<toml::Table>(src)
        .ok()
        .and_then(|mut t| t.remove("snippet"))
        .and_then(|v| v.try_into::<toml::Table>().ok())
        .unwrap_or_default();
    let (mut kept, mut refused) = (Vec::new(), Vec::new());
    for (name, v) in table {
        match v.as_str().filter(|t| snippet_ok(t).is_ok()) {
            Some(text) => kept.push((name, text.to_string())),
            None => refused.push(name),
        }
    }
    (kept, refused)
}

/// Every folder a device is in, innermost first: `acme/prod/db` is also in `acme/prod`
/// and in `acme`. The order is what decides a setting, so it is the order a reader
/// expects - the nearest folder speaks first, and the device's own list breaks a tie
/// between two branches it sits in.
fn folder_chain(folders: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in folders {
        let parts: Vec<&str> = f.split('/').filter(|p| !p.is_empty()).collect();
        for depth in (1..=parts.len()).rev() {
            let path = parts[..depth].join("/");
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    // A list naming a folder before one inside it (`["prod", "prod/eu"]`) would otherwise
    // let the outer one answer first. Stable, so two branches of the same depth keep the
    // order the device wrote them in.
    out.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));
    out
}

/// What a folder lends its devices. Only a setting the jack hasn't got itself.
fn under_folder(j: Jack, f: &Folder) -> Jack {
    Jack {
        user: j.user.or_else(|| f.user.clone()),
        port: j.port.or(f.port),
        key: j.key.or_else(|| f.key.clone()),
        jump: j.jump.or_else(|| f.jump.clone()),
        ..j
    }
}

/// Where a setting a device didn't set itself came from: a folder's path, or
/// `[defaults]`. Only the four a folder can lend, by jack name then key. The window
/// shows it, because a `user` nobody can trace to a line of the file is magic.
pub type Sources = IndexMap<String, IndexMap<String, String>>;

/// `[defaults]` merges into every jack; the jack's own value wins. A `[folder]` it is in
/// is asked in between: nearer than `[defaults]`, never louder than the device itself.
/// The second half is who answered, for everything the device left unsaid.
pub fn parse_all(src: &str) -> Result<(Jacks, Sources), String> {
    let raw: Raw = toml::from_str(src).map_err(|e| e.message().to_string())?;
    let d = &raw.defaults;
    let mut sources = Sources::new();
    let jacks = raw
        .jack
        .into_iter()
        .map(|(name, j)| {
            // Before the merge: a folder list that only `[defaults]` gives still says
            // which folders this device is in.
            let folders = j.folders.clone().or_else(|| d.folders.clone());
            let mut from = IndexMap::new();
            let mut note = |key: &str, mine: bool, theirs: bool, who: &str| {
                if !mine && theirs && !from.contains_key(key) {
                    from.insert(key.to_string(), who.to_string());
                }
            };
            let mut j = j;
            for path in folder_chain(folders.as_deref().unwrap_or_default()) {
                let Some(f) = raw.folder.get(&path) else {
                    continue;
                };
                note("user", j.user.is_some(), f.user.is_some(), &path);
                note("port", j.port.is_some(), f.port.is_some(), &path);
                note("key", j.key.is_some(), f.key.is_some(), &path);
                note("jump", j.jump.is_some(), f.jump.is_some(), &path);
                j = under_folder(j, f);
            }
            note("user", j.user.is_some(), d.user.is_some(), "defaults");
            note("port", j.port.is_some(), d.port.is_some(), "defaults");
            note("key", j.key.is_some(), d.key.is_some(), "defaults");
            note("jump", j.jump.is_some(), d.jump.is_some(), "defaults");
            sources.insert(name.clone(), from);

            let merged = Jack {
                name: j.name,
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
                telnet: j.telnet.or(d.telnet),
                // A cable is plugged into one machine for one device: never inherited.
                serial: j.serial,
                baud: j.baud.or(d.baud),
                ssh: j.ssh.or(d.ssh),
                primary: j.primary.or_else(|| d.primary.clone()),
                folders: j.folders.or_else(|| d.folders.clone()),
                desc: j.desc.or_else(|| d.desc.clone()),
                forward: j.forward.or_else(|| d.forward.clone()),
                mac: j.mac.or_else(|| d.mac.clone()),
                waits: false,
            };
            (name, merged)
        })
        .collect();
    Ok((jacks, sources))
}

/// Two vaults as one list: the team's devices under their own keys, yours under
/// `me:<key>`. Each was parsed on its own, so nothing inherits across. The one thing
/// that crosses is a jump, and only one way: a `jump` of yours that names one of your
/// own devices is pointed at it here, and anything else is left to find the team's.
/// `team` is `None` while the shared list is away.
pub fn merge(team: Option<Jacks>, mine: Jacks) -> Jacks {
    let away = team.is_none();
    // The one name the scheme can't tell apart, so a hand-written one is left out.
    let mut all: Jacks = team
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, _)| !k.starts_with(MINE))
        .collect();
    let own: HashSet<String> = mine.keys().cloned().collect();
    for (key, mut j) in mine {
        match j.jump.take() {
            Some(h) if own.contains(&h) => j.jump = Some(format!("{MINE}{h}")),
            Some(h) => {
                j.waits = away;
                j.jump = Some(h);
            }
            None => {}
        }
        all.insert(format!("{MINE}{key}"), j);
    }
    all
}

/// One step of a jump chain. A team device never finds one of yours, whatever its file
/// says: it has to behave the same on every machine the list is read on.
fn via<'a>(from: &str, h: &str, jacks: &'a Jacks) -> Option<&'a Jack> {
    (from.starts_with(MINE) || !h.starts_with(MINE))
        .then(|| jacks.get(h))
        .flatten()
}

/// A jump that found nothing is a raw ssh spec, unless the team list it may be in is
/// away: handing the name to ssh would dial whatever `~/.ssh/config` calls it.
fn raw_hop(from: &str, j: &Jack, h: String) -> Result<String, String> {
    if j.waits {
        return Err(format!(
            "\"{}\" jumps through \"{h}\", and the team list is not there",
            from.strip_prefix(MINE).unwrap_or(from)
        ));
    }
    dest(h)
}

/// A missing file is an empty list (a first run), not an error. A file that won't parse
/// still is one.
pub fn load(path: &Path) -> Result<Jacks, String> {
    load_all(path).map(|(jacks, _)| jacks)
}

pub fn load_all(path: &Path) -> Result<(Jacks, Sources), String> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    parse_all(&src).map_err(|e| format!("{}: {e}", path.display()))
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
    let (mut from, mut cur) = (name.to_string(), j);
    while let Some(h) = cur.jump.clone() {
        if !seen.insert(h.clone()) {
            return Err(format!("jump loop through \"{h}\""));
        }
        match via(&from, &h, jacks) {
            // Not a jack name: a raw ssh spec, passed through.
            None => {
                out.push(raw_hop(&from, cur, h)?);
                break;
            }
            Some(next) => {
                out.push(match next.port {
                    Some(p) => format!("{}:{p}", dest_of(next)?),
                    None => dest_of(next)?,
                });
                (from, cur) = (h, next);
            }
        }
    }
    out.reverse();
    Ok(out)
}

/// The machine the first TCP connection goes to: the outermost bastion, or the jack
/// itself. The only thing worth probing; anything past it is reachable only through ssh.
/// A hop as `-J` takes it, `user@host:port` with either end optional, as the host and
/// port a socket would be opened to.
/// ponytail: a bare IPv6 address reads its last group as a port; brackets if one shows.
fn host_port(h: &str) -> (String, u16) {
    let hp = h.rsplit('@').next().unwrap_or(h);
    match hp
        .rsplit_once(':')
        .and_then(|(a, b)| b.parse().ok().map(|p| (a, p)))
    {
        Some((host, port)) => (host.to_string(), port),
        None => (hp.to_string(), 22),
    }
}

pub fn entry(name: &str, jacks: &Jacks) -> Result<(String, u16), String> {
    let mut cur = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    if primary(cur) == "serial" {
        return Err(format!(
            "\"{name}\" is a serial console, with no address to try"
        ));
    }
    let mut seen: HashSet<String> = HashSet::from([name.to_string()]);
    let mut from = name.to_string();
    while let Some(h) = cur.jump.clone() {
        if !seen.insert(h.clone()) {
            return Err(format!("jump loop through \"{h}\""));
        }
        match via(&from, &h, jacks) {
            Some(next) => (from, cur) = (h, next),
            // The same refusal a connection gets: a name that may be the team's is not
            // probed as a host while their list is away.
            None if cur.waits => return raw_hop(&from, cur, h).map(|_| (String::new(), 0)),
            // A raw spec: user@host, host:port, or both.
            None => return Ok(host_port(&h)),
        }
    }
    Ok((cur.host.clone(), probe_port(cur)))
}

/// Which port the status dot tests. A device with `ssh = false` has nothing on 22.
fn probe_port(j: &Jack) -> u16 {
    if j.ssh.unwrap_or(true) {
        return j.port.unwrap_or(22);
    }
    if let Some(p) = j.rdp.or(j.vnc).or(j.telnet) {
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

/// The Wake on LAN packet for a hardware address: six `0xff`, then the address sixteen
/// times. Separators are `:`, `-` or `.`, or nothing, the ways a device's own label
/// prints it.
pub fn magic_packet(mac: &str) -> Result<Vec<u8>, String> {
    let hex: String = mac
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.'))
        .collect();
    // Byte indices below, so anything but ascii would slice through a character.
    let bytes: Option<Vec<u8>> = (hex.len() == 12 && hex.is_ascii())
        .then(|| {
            (0..12)
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                .collect()
        })
        .flatten();
    let mac = bytes.ok_or_else(|| format!("\"{mac}\" isn't a hardware address"))?;
    let mut packet = vec![0xff; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac);
    }
    Ok(packet)
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

/// One stop on the way to a device, and how to ask whether it answers. `ask` is the
/// ssh argv that asks from the stop before it; the first stop has none, because that
/// one is asked from here with a plain TCP connect.
#[derive(Debug, PartialEq)]
pub struct Stop {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub ask: Option<Vec<String>>,
}

/// The route to a device as questions that can each be answered on its own, in order,
/// so a failed connection can be pinned on one link. Each hop is asked the way the
/// connection itself reaches it: over the chain before it, with nothing typed, so a hop
/// that would need a password says so instead of waiting for one.
pub fn break_steps(name: &str, jacks: &Jacks) -> Result<Vec<Stop>, String> {
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    let hops = hops(name, jacks)?;
    let mut stops: Vec<Stop> = hops
        .iter()
        .map(|h| {
            let (host, port) = host_port(h);
            Stop {
                label: h.clone(),
                host,
                port,
                ask: None,
            }
        })
        .collect();
    stops.push(Stop {
        label: spec(j),
        host: j.host.clone(),
        port: probe_port(j),
        ask: None,
    });
    for k in 1..stops.len() {
        let from = &hops[k - 1];
        let (host, port) = (plain_host(&stops[k].host)?.to_string(), stops[k].port);
        let mut ask: Vec<String> = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=6"]
            .map(String::from)
            .to_vec();
        if k > 1 {
            ask.push("-J".into());
            ask.push(hops[..k - 1].join(","));
        }
        if stops[k - 1].port != 22 {
            ask.push("-p".into());
            ask.push(stops[k - 1].port.to_string());
        }
        // The destination without its port, which went into `-p`.
        let user = from.rsplit_once('@').map(|(u, _)| format!("{u}@"));
        ask.push(dest(format!(
            "{}{}",
            user.unwrap_or_default(),
            stops[k - 1].host
        ))?);
        // `plain_host` is what makes this safe to hand a remote shell as written. Both
        // ways of asking give up after three seconds: a firewall that drops the packet
        // is the case this exists for, and the kernel would wait two minutes on it.
        // ponytail: needs nc, or bash and timeout, on the hop; one with neither reads
        // as no answer. And a hop that is only a `Host` alias in `~/.ssh/config` can't
        // be asked about by name from another machine.
        ask.push(format!(
            "if command -v nc >/dev/null 2>&1; then nc -z -w 3 {host} {port}; \
             else timeout 3 bash -c 'exec 3<>/dev/tcp/{host}/{port}'; fi 2>/dev/null"
        ));
        stops[k].ask = Some(ask);
    }
    Ok(stops)
}

/// Ping or traceroute toward a device, as `(program, args)`. A device behind a jump
/// is not reachable from here, so the check runs *on the hop*, over the same chain.
/// `ping-on` is ping until ^C.
pub fn task_argv(task: &str, name: &str, jacks: &Jacks) -> Result<(String, Vec<String>), String> {
    if task == "key" {
        return key_argv(name, jacks);
    }
    if !["ping", "ping-on", "trace"].contains(&task) {
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
    // The same step a connection takes, so a ping can't reach what a session may not.
    let mut args = match via(name, hop, jacks) {
        Some(_) => without_forwards(ssh_args(hop, jacks)?),
        // A jump that isn't a jack is a raw ssh spec with no chain of its own.
        None => vec![raw_hop(name, j, hop.clone())?],
    };
    // Without a remote tty, ^C kills the local ssh and the far ping never prints its tally.
    if task == "ping-on" {
        args.insert(0, "-t".into());
    }
    args.extend(posix_task(task, host));
    Ok(("ssh".into(), args))
}

/// Make the device's key if this machine has none, then append its public half to the
/// device's `authorized_keys`. Both are the system's own tools on the pty, so the
/// passphrase and the device's password are asked in the tab and never reach patchbay.
/// The key is the device's `key`, so a shared list naming `~/.ssh/acme` gets everyone
/// their own `acme`. Every value is a positional parameter, never part of the script.
#[cfg(not(target_os = "windows"))]
fn key_argv(name: &str, jacks: &Jacks) -> Result<(String, Vec<String>), String> {
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    let key = expand(j.key.as_deref().unwrap_or("~/.ssh/id_ed25519"));
    let script = r#"k=$1; shift
[ -f "$k" ] || { mkdir -p -m 700 "$(dirname "$k")" && ssh-keygen -t ed25519 -f "$k"; } || exit
exec ssh-copy-id -i "$k" "$@""#;
    let mut args = vec!["-c".into(), script.into(), "sh".into(), key];
    if let Some(p) = j.port {
        args.extend(["-p".into(), p.to_string()]);
    }
    // ssh-copy-id has no -J of its own.
    let hop_list = hops(name, jacks)?;
    if !hop_list.is_empty() {
        args.extend(["-o".into(), format!("ProxyJump={}", hop_list.join(","))]);
    }
    args.push(dest_of(j)?);
    Ok(("sh".into(), args))
}

#[cfg(target_os = "windows")]
fn key_argv(_: &str, _: &Jacks) -> Result<(String, Vec<String>), String> {
    Err("installing a key needs ssh-copy-id, which Windows doesn't have".into())
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
        "ping-on" => vec!["ping".into(), host.into()],
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
        "ping-on" => ("ping".into(), vec!["-t".into(), host.into()]),
        _ => ("ping".into(), vec!["-n".into(), "5".into(), host.into()]),
    }
}

/// The `host` and `port` of a dialled `host:port`: the device, or the local end of a
/// forward to it.
pub fn split_addr(addr: &str) -> Result<(&str, u16), String> {
    addr.rsplit_once(':')
        .and_then(|(h, p)| p.parse::<u16>().ok().map(|p| (h, p)))
        .ok_or_else(|| format!("\"{addr}\" isn't a host and port"))
}

/// Telnet to `addr` as `(program, args)`. `have` says whether a program is on this
/// machine's PATH.
pub fn telnet_argv(
    addr: &str,
    have: impl Fn(&str) -> bool,
) -> Result<(String, Vec<String>), String> {
    let (host, port) = split_addr(addr)?;
    let host = plain_host(host)?;
    if have("telnet") {
        return Ok(("telnet".into(), vec![host.into(), port.to_string()]));
    }
    telnet_fallback(host, port)
}

/// curl speaks telnet, option negotiation and all, and ships on every desktop that lost
/// `telnet`. It reads its input as given, so the pty goes raw first, or every key would
/// wait for Enter and echo twice. The address is a parameter, never part of the script.
#[cfg(not(target_os = "windows"))]
fn telnet_fallback(host: &str, port: u16) -> Result<(String, Vec<String>), String> {
    Ok((
        "sh".into(),
        vec![
            "-c".into(),
            r#"stty raw -echo; exec curl --no-progress-meter "telnet://$1""#.into(),
            "sh".into(),
            format!("{host}:{port}"),
        ],
    ))
}

#[cfg(target_os = "windows")]
fn telnet_fallback(_: &str, _: u16) -> Result<(String, Vec<String>), String> {
    Err("telnet needs Windows' Telnet Client: Settings ▸ System ▸ Optional features".into())
}

/// The speeds a console port is set to. Anything else is a typo, and argv besides.
const BAUDS: [u32; 9] = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400];

/// A device's serial console as `(program, args)`: the first of `picocom`, `screen` and
/// `cu` this machine has. picocom first because it is made for this; `cu` last because
/// on macOS it wants a lock directory only root can write. `screenrc` is a file saying
/// `autodetach off` (`SCREENRC` below): closing the tab hangs up screen's terminal, and
/// by default it detaches on that and keeps the port busy with nobody attached.
pub fn serial_argv(
    j: &Jack,
    have: impl Fn(&str) -> bool,
    screenrc: &Path,
) -> Result<(String, Vec<String>), String> {
    let path = serial_path(j.serial.as_deref().unwrap_or_default())?;
    let baud = j.baud.unwrap_or(9600);
    if !BAUDS.contains(&baud) {
        return Err(format!("{baud} isn't a serial speed patchbay knows"));
    }
    if cfg!(target_os = "windows") {
        return Err(
            "a serial console needs picocom, screen or cu, which Windows doesn't have".into(),
        );
    }
    let b = baud.to_string();
    let p = path.to_string();
    if have("picocom") {
        Ok(("picocom".into(), vec!["-b".into(), b, p]))
    } else if have("screen") {
        let rc = screenrc.to_string_lossy().into_owned();
        Ok(("screen".into(), vec!["-c".into(), rc, p, b]))
    } else if have("cu") {
        Ok(("cu".into(), vec!["-l".into(), p, "-s".into(), b]))
    } else {
        Err("a serial console needs picocom, screen or cu installed".into())
    }
}

/// What `serial_argv` hands `screen` with `-c`.
pub const SCREENRC: &str = "autodetach off\n";

/// A serial device as argv will see it: a plain path under `/dev`, nothing that could be
/// read as an option or climb out.
pub fn serial_path(path: &str) -> Result<&str, String> {
    let path = path.trim();
    let ok = path.starts_with("/dev/")
        && !path.contains("..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c));
    ok.then_some(path)
        .ok_or_else(|| format!("\"{path}\" isn't a serial device: it has to be a /dev/ path"))
}

/// Whether `program` is on this machine's PATH.
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|d| {
            let f = d.join(program);
            f.is_file() || (cfg!(windows) && f.with_extension("exe").is_file())
        })
    })
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

/// How a device is reached: "ssh", "rdp", "vnc", "telnet", "serial" or "web". `primary`
/// is still read for configs that set several, and ignored when it names something the
/// device lost.
pub fn primary(j: &Jack) -> String {
    // "sftp" is ssh with a different default action.
    let has = |k: &str| match k {
        "ssh" | "sftp" => j.ssh.unwrap_or(true),
        "rdp" => j.rdp.is_some(),
        "vnc" => j.vnc.is_some(),
        "telnet" => j.telnet.is_some(),
        "serial" => j.serial.as_deref().is_some_and(|s| !s.trim().is_empty()),
        _ => j.url.is_some(),
    };
    j.primary
        .as_deref()
        .filter(|p| has(p))
        .map(str::to_string)
        .unwrap_or_else(|| {
            ["ssh", "rdp", "vnc", "telnet", "serial", "web"]
                .iter()
                .find(|k| has(k))
                .unwrap_or(&"ssh")
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The merge, for a test that doesn't care who lent what.
    fn jacks_of(src: &str) -> Result<Jacks, String> {
        parse_all(src).map(|(jacks, _)| jacks)
    }

    const TEAM: &str = r#"
[jack.bastion]
host = "10.0.0.2"
port = 2222
[jack.web]
host = "10.0.0.4"
jump = "bastion"
[jack.db]
host = "10.0.0.5"
jump = "nas"
[jack.odd]
host = "10.0.0.6"
jump = "me:nas"
"#;
    const OWN: &str = r#"
[defaults]
user = "me"
[jack.nas]
host = "10.0.1.20"
[jack.pi]
host = "10.0.1.30"
jump = "nas"
[jack.vm]
host = "10.0.0.41"
jump = "web"
[jack.bastion]
host = "10.0.1.2"
"#;

    #[test]
    fn a_route_is_asked_one_link_at_a_time_over_the_chain_before_it() {
        let jacks = jacks_of(
            "[jack.edge]\nhost = \"10.0.0.2\"\nport = 2222\nuser = \"ops\"\n\
             [jack.mid]\nhost = \"10.0.0.3\"\njump = \"edge\"\n\
             [jack.db]\nhost = \"10.0.0.5\"\njump = \"mid\"\n\
             [jack.solo]\nhost = \"10.0.0.9\"\n",
        )
        .unwrap();
        let s = break_steps("db", &jacks).unwrap();
        assert_eq!(
            s.iter()
                .map(|x| (x.host.as_str(), x.port))
                .collect::<Vec<_>>(),
            [("10.0.0.2", 2222), ("10.0.0.3", 22), ("10.0.0.5", 22)]
        );
        // The first hop is asked from here; nothing is run for it.
        assert_eq!(s[0].ask, None);
        // The second is asked from the first, on the first's own port.
        let a = s[1].ask.as_ref().unwrap();
        assert_eq!(a[4..7], ["-p", "2222", "ops@10.0.0.2"]);
        assert!(a[7].contains("nc -z -w 3 10.0.0.3 22;"), "{}", a[7]);
        // The third from the second, through the first.
        let b = s[2].ask.as_ref().unwrap();
        assert_eq!(b[4..7], ["-J", "ops@10.0.0.2:2222", "10.0.0.3"]);
        assert!(
            b[7].contains("timeout 3 bash -c 'exec 3<>/dev/tcp/10.0.0.5/22'"),
            "{}",
            b[7]
        );
        // With no chain there is one question, and it is asked from here.
        assert_eq!(break_steps("solo", &jacks).unwrap().len(), 1);
    }

    #[test]
    fn a_snippet_can_never_press_a_key() {
        assert!(snippet_ok("sudo systemctl restart nginx").is_ok());
        assert!(snippet_ok("grep -r 'ä' /srv | head").is_ok());
        for bad in [
            "ls\n", "ls\r", "a\tb", "\x03", "\x1b[2J", "x\u{7f}", "x\u{85}", "", "  ",
        ] {
            assert!(snippet_ok(bad).is_err(), "{bad:?}");
        }
        assert!(snippet_ok(&"x".repeat(1001)).is_err());

        let (kept, refused) = snippets(
            "[snippet]\ndisk = \"df -h /\"\n\"two steps\" = \"cd /srv\\nls\"\nodd = 3\nlast = \"w\"\n",
        );
        assert_eq!(
            kept,
            [
                ("disk".to_string(), "df -h /".to_string()),
                ("last".into(), "w".into())
            ]
        );
        assert_eq!(refused, ["two steps", "odd"]);
    }

    #[test]
    fn two_vaults_share_a_key_and_a_jump_only_crosses_one_way() {
        let all = merge(Some(jacks_of(TEAM).unwrap()), jacks_of(OWN).unwrap());
        assert_eq!(all["bastion"].host, "10.0.0.2");
        assert_eq!(all["me:bastion"].host, "10.0.1.2");
        // Yours through one of yours.
        assert_eq!(hops("me:pi", &all).unwrap(), ["me@10.0.1.20"]);
        // Yours through the team's, three deep: their bastion keeps its own port and
        // none of your `[defaults]`, and the chain never comes back for your `bastion`.
        assert_eq!(hops("me:vm", &all).unwrap(), ["10.0.0.2:2222", "10.0.0.4"]);
        // The team's never finds yours: the name goes to ssh as written, as it does on
        // every other machine the list is read on.
        assert_eq!(hops("db", &all).unwrap(), ["nas"]);
        assert_eq!(hops("odd", &all).unwrap(), ["me:nas"]);
        assert_eq!(entry("db", &all).unwrap(), ("nas".to_string(), 22));
        let (_, ping) = task_argv("ping", "odd", &all).unwrap();
        assert_eq!(ping[0], "me:nas", "a team ping went through one of yours");
    }

    #[test]
    fn with_the_team_list_away_yours_still_connect_unless_they_jump_into_it() {
        let all = merge(None, jacks_of(OWN).unwrap());
        assert!(!all.contains_key("web"));
        assert_eq!(hops("me:pi", &all).unwrap(), ["me@10.0.1.20"]);
        let err = ssh_args("me:vm", &all).unwrap_err();
        assert_eq!(
            err,
            r#""vm" jumps through "web", and the team list is not there"#
        );
        // A ping and the status dot's probe are refused the same way.
        assert_eq!(task_argv("ping", "me:vm", &all).unwrap_err(), err);
        assert_eq!(entry("me:vm", &all).unwrap_err(), err);
    }

    #[test]
    fn a_team_key_shaped_like_one_of_yours_is_left_out() {
        let team = jacks_of("[jack.\"me:x\"]\nhost = \"10.0.0.9\"\n").unwrap();
        assert!(merge(Some(team), Jacks::new()).is_empty());
    }

    #[test]
    fn the_list_setting_names_the_shared_file_and_expands_home() {
        let own = std::env::temp_dir().join(format!("patchbay-list-{}.toml", std::process::id()));
        std::fs::write(&own, "[settings]\nlist = \"~/team/patchbay.toml\"\n").unwrap();
        let l = list_at(&own).unwrap();
        assert!(!l.starts_with("~"));
        assert!(l.ends_with("team/patchbay.toml"));
        std::fs::write(&own, "[settings]\nlist = \"  \"\n[jack.a]\nhost = \"h\"\n").unwrap();
        assert_eq!(list_at(&own), None, "blank is unset");
        std::fs::remove_file(&own).unwrap();
    }

    #[test]
    fn a_folder_lends_its_settings_and_the_nearest_one_wins() {
        let (jacks, from) = parse_all(
            r#"
            [defaults]
            user = "root"

            [folder.acme]
            jump = "acme-gw"
            user = "admin"

            [folder."acme/prod"]
            key = "~/.ssh/acme-prod"
            user = "ops"

            [jack.db]
            host = "10.0.0.5"
            folders = ["acme/prod"]

            [jack.pdc]
            host = "10.0.0.6"
            user = "own"
            folders = ["acme"]

            [jack.elsewhere]
            host = "10.0.0.7"

            # Filed in a folder and in its parent, parent first: the inner one still wins.
            [jack.both]
            host = "10.0.0.8"
            folders = ["acme", "acme/prod"]
            "#,
        )
        .unwrap();

        let db = &jacks["db"];
        assert_eq!(db.user.as_deref(), Some("ops"), "the nearest folder speaks");
        assert_eq!(db.key.as_deref(), Some("~/.ssh/acme-prod"));
        assert_eq!(db.jump.as_deref(), Some("acme-gw"), "and the one above it");
        assert_eq!(jacks["pdc"].user.as_deref(), Some("own"), "the device wins");
        assert_eq!(jacks["pdc"].jump.as_deref(), Some("acme-gw"));
        assert_eq!(
            jacks["elsewhere"].user.as_deref(),
            Some("root"),
            "a device in no folder is left to [defaults]"
        );
        assert_eq!(jacks["elsewhere"].jump, None);
        assert_eq!(
            jacks["both"].user.as_deref(),
            Some("ops"),
            "the outer folder answered over the one inside it"
        );

        // And the pane can say where each of those came from. A device's own value is
        // nobody else's, so it is named by nobody.
        assert_eq!(from["db"]["user"], "acme/prod");
        assert_eq!(from["db"]["jump"], "acme");
        assert_eq!(
            from["pdc"].get("user"),
            None,
            "its own user, not the folder's"
        );
        assert_eq!(from["pdc"]["jump"], "acme");
        assert_eq!(from["elsewhere"]["user"], "defaults");
        assert_eq!(from["elsewhere"].get("jump"), None, "nobody set one");
    }

    /// A folder is written in the same file a colleague writes, so what it lends is as
    /// untrusted as a device's own - and it reaches argv by the same road.
    #[test]
    fn a_folder_cannot_lend_an_ssh_option_either() {
        let j = jacks_of(
            r#"
            [folder.acme]
            jump = "-oProxyCommand=touch /tmp/pwned"

            [jack.db]
            host = "10.0.0.5"
            folders = ["acme"]
            "#,
        )
        .unwrap();
        assert!(ssh_args("db", &j).is_err(), "a lent jump reached argv");
    }

    #[test]
    fn a_hardware_address_becomes_a_magic_packet_however_it_is_written() {
        let want = magic_packet("001b:2192.0A1f").unwrap();
        assert_eq!(want.len(), 102);
        assert_eq!(&want[..6], &[0xff; 6]);
        assert_eq!(&want[6..12], &[0x00, 0x1b, 0x21, 0x92, 0x0a, 0x1f]);
        assert_eq!(&want[96..], &want[6..12]);
        for same in ["00:1b:21:92:0a:1f", "00-1B-21-92-0A-1F", "001B21920A1F"] {
            assert_eq!(magic_packet(same).unwrap(), want, "{same}");
        }
        for bad in [
            "",
            "00:1b:21:92:0a",
            "zz:1b:21:92:0a:1f",
            "00:1b:21:92:0a:1f:2c",
            // Twelve bytes, but not twelve characters: the hex is sliced by byte.
            "0é0123456789",
        ] {
            assert!(magic_packet(bad).is_err(), "{bad} was accepted");
        }
    }

    fn fixture() -> Jacks {
        jacks_of(
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
        let j = jacks_of(
            r#"
[folder.acme]
user = "root"

[jack.bastion]
host = "bastion.example"
port = 2222
folders = ["acme"]

[jack.web]
host = "10.0.0.4"
user = "deploy"
jump = "bastion"
folders = ["acme"]

[jack.db]
host = "10.0.0.5"
jump = "web"
folders = ["acme"]
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
        let j = jacks_of(
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
        let j = jacks_of(
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
        let j = jacks_of(
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
        let dressed =
            jacks_of("[jack.x]\nhost = \"-oProxyCommand=id\"\nuser = \"root\"\n").unwrap();
        assert!(ssh_args("x", &dressed).is_err());
        let fine = jacks_of("[jack.x]\nhost = \"10.0.0.4\"\nuser = \"root\"\n").unwrap();
        assert_eq!(
            ssh_args("x", &fine).unwrap().last().unwrap(),
            "root@10.0.0.4"
        );
    }

    #[test]
    fn every_kind_of_forward_reaches_argv_behind_its_own_flag() {
        let j = jacks_of(
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
        let bad = jacks_of("[jack.x]\nhost = \"h\"\nforward = [\"-o ProxyCommand=id\"]\n").unwrap();
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
        let j = jacks_of(CHAIN).unwrap();

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

        let (_, a) = task_argv("ping-on", "db", &j).unwrap();
        assert_eq!(a[0], "-t", "^C has to reach the far ping");
        assert_eq!(a[a.len() - 2..], ["ping", "db.internal"]);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_key_goes_over_the_same_chain_and_never_into_the_script() {
        let j = jacks_of(CHAIN).unwrap();
        let (p, a) = task_argv("key", "db", &j).unwrap();
        assert_eq!(p, "sh");
        assert_eq!(a[2], "sh", "$0, so the key is $1");
        assert!(a[3].ends_with("/.ssh/id_ed25519"), "got {a:?}");
        assert!(
            !a[1].contains("db.internal"),
            "values stay out of the script"
        );
        assert_eq!(
            a[4..],
            ["-o", "ProxyJump=ops@bastion.example:2222", "db.internal"]
        );
    }

    #[test]
    fn a_host_that_could_be_a_command_on_the_hop_is_refused() {
        let j = jacks_of(CHAIN).unwrap();
        assert!(
            task_argv("ping", "sneaky", &j).is_err(),
            "a space reaches the hop's shell"
        );
        assert!(
            task_argv("nope", "plain", &j).is_err(),
            "only the three tasks exist"
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
            // What a login page navigates through on its way, and what `on_navigation`
            // must not report as a page that failed to open.
            "about:blank",
            "blob:https://nas.local/9f2b",
            "",
            "https://ok\nfile:///etc/passwd",
        ] {
            assert!(!is_web_url(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn telnet_and_serial_are_the_systems_tools_and_take_nothing_unchecked() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(
            telnet_argv("10.0.0.20:23", all).unwrap(),
            ("telnet".into(), vec!["10.0.0.20".into(), "23".into()])
        );
        // A host that would be an option, or a shell's business on the way through curl.
        assert!(telnet_argv("-oProxy:23", all).is_err());
        assert!(telnet_argv("a;b:23", none).is_err());
        assert!(telnet_argv("10.0.0.20", all).is_err());
        #[cfg(not(target_os = "windows"))]
        {
            let (prog, args) = telnet_argv("127.0.0.1:40001", none).unwrap();
            assert_eq!(prog, "sh");
            // The address is a parameter after the script, never inside it.
            assert_eq!(args.last().unwrap(), "127.0.0.1:40001");
            assert!(!args[1].contains("40001"));
        }

        let console = |serial: &str, baud: Option<u32>| Jack {
            serial: Some(serial.into()),
            baud,
            ssh: Some(false),
            ..Jack::default()
        };
        assert_eq!(primary(&console("/dev/cu.usbserial-1410", None)), "serial");
        let rc = Path::new("/tmp/patchbay-screenrc");
        assert!(serial_argv(&console("/dev/../etc/passwd", None), all, rc).is_err());
        assert!(serial_argv(&console("-l/dev/x", None), all, rc).is_err());
        assert!(serial_argv(&console("/dev/tty S0", None), all, rc).is_err());
        assert!(serial_argv(&console("/dev/cu.x", Some(12345)), all, rc).is_err());
        #[cfg(not(target_os = "windows"))]
        {
            let has = |p: &'static str| move |q: &str| q == p;
            let j = console("/dev/cu.usbserial-1410", Some(115200));
            assert_eq!(
                serial_argv(&j, has("picocom"), rc).unwrap().1,
                ["-b", "115200", "/dev/cu.usbserial-1410"]
            );
            // With an rc that keeps screen from outliving its tab on the port.
            assert_eq!(
                serial_argv(&j, has("screen"), rc).unwrap().1,
                [
                    "-c",
                    "/tmp/patchbay-screenrc",
                    "/dev/cu.usbserial-1410",
                    "115200"
                ]
            );
            assert_eq!(serial_argv(&j, has("cu"), rc).unwrap().0, "cu");
            assert!(serial_argv(&j, none, rc).is_err());
        }
    }

    #[test]
    fn a_serial_console_is_never_probed_and_telnet_is_probed_on_its_port() {
        let jacks = jacks_of(
            r#"
[jack.console]
serial = "/dev/cu.usbserial-1410"
ssh = false
[jack.switch]
host = "10.0.0.20"
telnet = 23
ssh = false
"#,
        )
        .unwrap();
        assert!(entry("console", &jacks).is_err());
        assert_eq!(entry("switch", &jacks).unwrap(), ("10.0.0.20".into(), 23));
        assert_eq!(primary(&jacks["switch"]), "telnet");
    }
}
