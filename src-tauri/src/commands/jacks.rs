//! The device list: reading it for the window, editing it, folders and notes, and
//! importing one from an ssh config or a Royal TS document.

use super::remote::rdp_pin;
use super::{blocking, file_for, file_of, list_file, load_all, load_jacks, os_open, ssh_dir};
use crate::{config, hostkeys, import, keychain, patchbay, presence, terminal};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct JackView {
    /// The key: what jumps, links and every command go by.
    name: String,
    /// What the window shows.
    label: String,
    host: String,
    user: Option<String>,
    port: Option<u16>,
    key: Option<String>,
    /// As the file says it, which is what the sheet edits and writes back.
    jump: Option<String>,
    /// "mine" or "team" once a shared list is set; absent while there is one vault.
    vault: Option<&'static str>,
    os: Option<String>,
    url: Option<String>,
    rdp: Option<u16>,
    vnc: Option<u16>,
    telnet: Option<u16>,
    serial: Option<String>,
    baud: Option<u32>,
    ssh: bool,
    primary: String,
    desc: Option<String>,
    mac: Option<String>,
    folders: Vec<String>,
    forward: Vec<String>,
    /// Where a setting the device didn't set itself came from: the folder's path, or
    /// "defaults". The pane prints it beside the value.
    from: indexmap::IndexMap<String, String>,
    /// Ordered hops, nearest first: what the detail pane draws as the route.
    hops: Vec<String>,
    /// The ssh command line, shown so you always see what is about to run.
    command: String,
    /// The table as read, sent back with an edit so a colleague's change in between
    /// is refused rather than overwritten.
    stamp: Option<String>,
}

#[derive(Serialize)]
pub struct Probe {
    name: String,
    target: String,
    /// TCP connect time in ms, or null if it didn't answer inside the timeout.
    ms: Option<u64>,
}

/// One folder's own row: what is written about it, and what it lends its devices.
#[derive(Serialize)]
pub struct Note {
    path: String,
    note: String,
    /// What the folder lends every device in it. Its own `note` is never in here; the
    /// window edits the two in different places.
    defaults: patchbay::Folder,
    stamp: String,
    vault: Option<&'static str>,
}

/// The devices, and what the window needs to know about the vaults they are in. One
/// answer, because both come out of the same read of the same files.
#[derive(Serialize)]
pub struct Listing {
    jacks: Vec<JackView>,
    vaults: Vaults,
}

/// What the window needs to know about the vaults besides the devices in them.
#[derive(Serialize)]
pub struct Vaults {
    /// A shared list is set, so there are two.
    two: bool,
    /// Where the team's list is, for the sidebar to name.
    team: Option<String>,
    /// Why the team's half is missing, when it is.
    away: Option<String>,
    /// Devices of your own that are unchanged copies of the team's.
    copies: usize,
    /// The team licence, for the sidebar to say where one is needed or has ended. Read
    /// even when the list was refused, since the licence is why.
    licence: Option<crate::licence::Status>,
    /// Whether licences are on sale, so the window asks for one only once they are.
    on_sale: bool,
}

#[derive(Serialize)]
pub struct Snippet {
    name: String,
    text: String,
    vault: Option<&'static str>,
}

/// Both vaults' snippets, yours first, and what the sheet needs to save them back.
#[derive(Serialize)]
pub struct Snippets {
    list: Vec<Snippet>,
    /// Names left out because their text was not one plain line.
    refused: Vec<String>,
    /// The `[snippet]` table of each vault as read, by the vault's word.
    stamps: std::collections::HashMap<&'static str, String>,
}

/// A delete, with the vault it came out of so Undo puts it back there.
#[derive(Serialize, serde::Deserialize)]
pub struct Gone {
    #[serde(flatten)]
    removed: config::Removed,
    vault: Option<String>,
}

/// Which vault a name is in, as the word the window knows it by.
fn vault_of(name: &str) -> Option<&'static str> {
    patchbay::list().map(|_| match name.starts_with(patchbay::MINE) {
        true => "mine",
        false => "team",
    })
}

/// What the poll compares: the bytes, hashed. Not "newer" - a sync client keeps the
/// source machine's mtime and clocks disagree - and not the size either, which doesn't
/// move when someone's `port = 2222` becomes `2223`. `conflict` names a copy a sync
/// client left beside the list when two people saved in the same minute.
#[derive(Serialize)]
pub struct ListStamp {
    stamp: String,
    conflict: Option<String>,
}

#[derive(Serialize)]
pub struct SshHosts {
    path: String,
    hosts: Vec<import::Imported>,
    warnings: Vec<String>,
}

/// Keep `~/.ssh/patchbay.conf` current when the setting asks for it. Done here rather
/// than in every writer because a hand-edit changes the list without any writer running.
/// Quiet on failure: an unwritable home directory is no reason to fail the list.
fn sync_ssh_config(jacks: &patchbay::Jacks) {
    let settings = config::load_settings();
    if !settings.write_ssh_config {
        return;
    }
    // With one file there is no `me:` to tell the vaults apart, and every device goes.
    let mine = settings.ssh_config_vault == "mine" && patchbay::list().is_some();
    let Some(dir) = ssh_dir() else { return };
    let theirs = std::fs::read_to_string(dir.join("config"))
        .map(|s| import::host_names(&s))
        .unwrap_or_default();
    let (named, left_out) = import::one_vault(jacks, mine);
    let _ = config::write_ssh_include(&dir, &import::to_ssh_config(&named, &theirs, left_out));
}

#[tauri::command]
pub async fn jacks() -> Result<Listing, String> {
    blocking(|| {
        let super::Loaded {
            jacks,
            mut sources,
            away,
        } = load_all()?;
        sync_ssh_config(&jacks);
        let shared = patchbay::list();
        let own = config::stamps_at(&patchbay::config_path(), "jack");
        // An away list has no rows to stamp, so its path is never asked for.
        let team = match &shared {
            Some(p) if away.is_none() => config::stamps_at(p, "jack"),
            _ => Default::default(),
        };
        let views = jacks
            .iter()
            .map(|(name, j)| {
                let vault = vault_of(name);
                // Only with two vaults is `me:` ours to take off; with one it is
                // whatever somebody wrote.
                let bare = |s: &'_ str| -> String {
                    match vault {
                        Some("mine") => s.strip_prefix(patchbay::MINE).unwrap_or(s).to_string(),
                        _ => s.to_string(),
                    }
                };
                let key = bare(name);
                JackView {
                    stamp: if vault == Some("team") { &team } else { &own }
                        .get(&key)
                        .cloned(),
                    name: name.clone(),
                    label: j.name.clone().unwrap_or(key),
                    host: j.host.clone(),
                    user: j.user.clone(),
                    port: j.port,
                    jump: j.jump.as_deref().map(bare),
                    vault,
                    os: j.os.clone(),
                    url: j.url.clone(),
                    rdp: j.rdp,
                    vnc: j.vnc,
                    telnet: j.telnet,
                    serial: j.serial.clone(),
                    baud: j.baud,
                    ssh: j.ssh.unwrap_or(true),
                    primary: patchbay::primary(j),
                    desc: j.desc.clone(),
                    mac: j.mac.clone(),
                    key: j.key.clone(),
                    from: sources.swap_remove(name).unwrap_or_default(),
                    folders: j.folders.clone().unwrap_or_default(),
                    forward: j.forward.clone().unwrap_or_default(),
                    hops: patchbay::hops(name, &jacks).unwrap_or_default(),
                    command: patchbay::ssh_args(name, &jacks)
                        .map(|a| terminal::command_line(&a))
                        .unwrap_or_else(|e| e),
                }
            })
            .collect();
        Ok(Listing {
            jacks: views,
            vaults: Vaults {
                two: shared.is_some(),
                // The same test `copies_at` makes, on the stamps already in hand.
                copies: own.iter().filter(|(k, s)| team.get(*k) == Some(s)).count(),
                licence: shared
                    .as_deref()
                    .filter(|p| crate::licence::ON_SALE && p.is_file())
                    .map(crate::licence::status),
                on_sale: crate::licence::ON_SALE,
                team: shared.map(|p| p.display().to_string()),
                away,
            },
        })
    })
    .await
}

/// The copies a seeded list left in your own vault, removed. Only tables that are
/// still the same on both sides; the file as it was is kept beside it.
#[tauri::command]
pub async fn remove_copies() -> Result<usize, String> {
    blocking(|| config::remove_copies_at(&patchbay::config_path(), &list_file()?)).await
}

/// Cheap enough for a timer: one read of a file that is kilobytes, and one directory
/// listing when the list is shared. Dropbox writes `x (conflicted copy ...)`, OneDrive
/// `x-MACHINE`; anything else with the list's stem and extension in that directory is
/// close enough to name.
#[tauri::command]
pub async fn list_stamp() -> Result<ListStamp, String> {
    blocking(|| {
        use std::hash::{Hash, Hasher};
        // Both vaults in one number: a change to either is a change to the list. The
        // team's is read if it is there and hashed as missing if not, so the poll also
        // notices a share coming back.
        let mut h = std::hash::DefaultHasher::new();
        if patchbay::list().is_some() {
            std::fs::read(patchbay::config_path()).ok().hash(&mut h);
            list_file()
                .ok()
                .and_then(|p| std::fs::read(p).ok())
                .hash(&mut h);
        } else {
            let path = list_file()?;
            let src = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            src.hash(&mut h);
        }
        let conflict = patchbay::list().and_then(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            let mine = p.file_name()?.to_os_string();
            std::fs::read_dir(p.parent()?)
                .ok()?
                .flatten()
                .map(|e| e.file_name())
                .filter(|n| *n != mine)
                .filter_map(|n| n.into_string().ok())
                .find(|n| n.starts_with(&stem) && n.ends_with(".toml"))
        });
        Ok(ListStamp {
            stamp: h.finish().to_string(),
            conflict,
        })
    })
    .await
}

/// The folder the list is in, for the pill that names a conflicted copy.
#[tauri::command]
pub fn reveal_list() -> Result<(), String> {
    let p = list_file()?;
    os_open(p.parent().unwrap_or(&p).as_os_str())
}

/// TCP-connect every device's entry point in parallel. Nothing is sent. Each distinct
/// host:port is dialled once - twenty devices behind one bastion are one connection -
/// and at most `POOL` at a time, so a thousand devices don't mean a thousand threads.
#[tauri::command]
pub async fn probe() -> Result<Vec<Probe>, String> {
    blocking(|| {
        let jacks = load_jacks()?;
        let targets: Vec<(String, String, u16)> = jacks
            .keys()
            .filter_map(|n| {
                patchbay::entry(n, &jacks)
                    .ok()
                    .map(|(h, p)| (n.clone(), h, p))
            })
            .collect();
        let distinct: Vec<(String, u16)> = targets
            .iter()
            .map(|(_, h, p)| (h.clone(), *p))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let timed = probe_all(&distinct);
        Ok(targets
            .into_iter()
            .map(|(name, host, port)| Probe {
                target: format!("{host}:{port}"),
                ms: timed.get(&(host, port)).copied().flatten(),
                name,
            })
            .collect())
    })
    .await
}

/// Whether one device's entry point takes a TCP connection: what a dead tab waits on
/// before it dials again. Behind a bastion that is the bastion, so the redial itself
/// is the real answer there.
#[tauri::command]
pub async fn reachable(name: String) -> Result<bool, String> {
    blocking(move || {
        let (host, port) = patchbay::entry(&name, &load_jacks()?)?;
        Ok(connect_ms(&host, port).is_some())
    })
    .await
}

/// One stop of a route after it was asked: `ok` is absent for the ones past the break,
/// which nobody could ask.
#[derive(Serialize)]
pub struct Asked {
    label: String,
    ok: Option<bool>,
    said: String,
}

/// Walk the route to a device one link at a time and say where it stops. Reads only:
/// a TCP connect from here, then each hop asked whether the next one answers.
#[tauri::command]
pub async fn find_break(name: String) -> Result<Vec<Asked>, String> {
    blocking(move || {
        let stops = patchbay::break_steps(&name, &load_jacks()?)?;
        // The team's host keys are deliberately not offered here. Each of these ssh runs
        // ends at a *hop*, and the check that keeps the team's file from outvoting this
        // machine's own key is made per host: made for the device and applied to a hop,
        // it would let a planted key for the bastion pass. A hop this machine has never
        // connected to reads as "could not log in", which is true of it.
        let mut out: Vec<Asked> = Vec::new();
        let mut broken = false;
        for stop in &stops {
            let at = format!("port {}", stop.port);
            let (ok, said) = match (&stop.ask, broken) {
                (_, true) => (None, String::new()),
                (None, _) => match connect_ms(&stop.host, stop.port) {
                    Some(ms) => (Some(true), format!("answers from here on {at}, {ms}ms")),
                    None => (Some(false), format!("no answer from this machine on {at}")),
                },
                (Some(argv), _) => {
                    let ran = std::process::Command::new("ssh")
                        .args(argv)
                        .stdin(std::process::Stdio::null())
                        .output()
                        .map_err(|e| format!("could not run ssh: {e}"))?;
                    match ran.status.code() {
                        Some(0) => (Some(true), format!("answers on {at} from the hop before")),
                        // ssh's own failure: the hop before answered, and would not let us in.
                        Some(255) => {
                            let why = String::from_utf8_lossy(&ran.stderr)
                                .lines()
                                .last()
                                .unwrap_or("ssh could not log in")
                                .trim()
                                .to_string();
                            if let Some(prev) = out.last_mut() {
                                prev.ok = Some(false);
                                prev.said = format!("it answers, but ssh could not log in: {why}");
                            }
                            (None, String::new())
                        }
                        _ => (
                            Some(false),
                            format!("no answer on {at} from the hop before"),
                        ),
                    }
                }
            };
            broken = broken || ok != Some(true);
            out.push(Asked {
                label: stop.label.clone(),
                ok,
                said,
            });
        }
        Ok(out)
    })
    .await
}

/// A team device's host as `known_hosts` names it.
fn key_host(name: &str) -> Result<String, String> {
    let jacks = load_jacks()?;
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    Ok(hostkeys::spec(&j.host, j.port))
}

/// What this machine and the team each know of a device's host key, as fingerprints.
#[tauri::command]
pub async fn host_key(name: String) -> Result<hostkeys::Known, String> {
    blocking(move || Ok(hostkeys::known(&key_host(&name)?))).await
}

/// Put the key this machine has for a device into the team's file.
#[tauri::command]
pub async fn share_host_key(name: String) -> Result<usize, String> {
    blocking(move || hostkeys::share(&key_host(&name)?)).await
}

/// Which team devices this window has a session on. Sent again on a timer: the file's
/// age is the heartbeat. Async for the share's sake, like everything that touches it.
#[tauri::command]
pub async fn presence_set(names: Vec<String>) -> Result<(), String> {
    blocking(move || {
        presence::set(&names);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn presence() -> Result<Vec<presence::Seen>, String> {
    blocking(|| Ok(presence::others())).await
}

const POOL: usize = 64;

/// A bounded pool over a shared cursor: the next free thread takes the next target.
fn probe_all(targets: &[(String, u16)]) -> std::collections::HashMap<(String, u16), Option<u64>> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new(std::collections::HashMap::new());
    std::thread::scope(|s| {
        for _ in 0..POOL.min(targets.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((host, port)) = targets.get(i) else {
                    break;
                };
                let ms = connect_ms(host, *port);
                out.lock().unwrap().insert((host.clone(), *port), ms);
            });
        }
    });
    out.into_inner().unwrap()
}

fn connect_ms(host: &str, port: u16) -> Option<u64> {
    use std::net::ToSocketAddrs;
    let started = std::time::Instant::now();
    let addr = format!("{host}:{port}").to_socket_addrs().ok()?.next()?;
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).ok()?;
    Some(started.elapsed().as_millis() as u64)
}

// Every command below is async: a sync command runs on the main thread, and a stat on
// a share that has gone away hangs for tens of seconds. The window must not.

/// `vault` is where a new device goes, and on an edit where it should end up: a
/// different one from where it is moves it, after the edit has been saved in place.
#[tauri::command]
pub async fn save_jack(
    original: Option<String>,
    jack: config::JackInput,
    vault: Option<String>,
) -> Result<(), String> {
    blocking(move || {
        let was = original.as_deref().and_then(vault_of);
        let to = vault.as_deref().or(was);
        let (file, key) = match original.as_deref() {
            Some(o) => file_for(o).map(|(f, k)| (f, Some(k)))?,
            None => (file_of(to)?, None),
        };
        if to == Some("team") {
            team_jump(jack.name.trim(), jack.jump.as_deref())?;
        }
        // Asked before the edit is written: a move refused afterwards would leave the
        // sheet reporting an error for a save that half happened.
        if let (Some(a), Some(b), Some(k)) = (was, to, key.as_deref()) {
            if a != b {
                free_in(b, k)?;
            }
        }
        let key = config::save_jack_at(&file, key, jack)?;
        match (was, to) {
            (Some(a), Some(b)) if a != b => move_between(a, &key),
            _ => Ok(()),
        }
    })
    .await
}

/// A team device has to reach the same machine from everyone's: a jump that only
/// names something in your own vault would work here and nowhere else.
fn team_jump(name: &str, jump: Option<&str>) -> Result<(), String> {
    let Some(h) = jump.map(str::trim).filter(|h| !h.is_empty()) else {
        return Ok(());
    };
    let h = h.strip_prefix(patchbay::MINE).unwrap_or(h);
    let has = |file: &std::path::Path| patchbay::load(file).is_ok_and(|j| j.contains_key(h));
    if !has(&list_file()?) && has(&patchbay::config_path()) {
        return Err(format!(
            "\"{name}\" jumps through \"{h}\", which is only on this machine"
        ));
    }
    Ok(())
}

/// A key is what jumps and links go by, so a move never changes it: the other vault
/// has to have it free.
fn free_in(vault: &str, key: &str) -> Result<(), String> {
    if patchbay::load(&file_of(Some(vault))?)?.contains_key(key) {
        let whose = if vault == "mine" {
            "your own list"
        } else {
            "the team list"
        };
        return Err(format!(
            "there's already a \"{key}\" in {whose} - rename one of them first"
        ));
    }
    Ok(())
}

/// `from` is the vault `key` is in now.
fn move_between(from: &str, key: &str) -> Result<(), String> {
    let (own, team) = (patchbay::config_path(), list_file()?);
    free_in(if from == "mine" { "team" } else { "mine" }, key)?;
    if from == "mine" {
        let jump = patchbay::load(&own)?.get(key).and_then(|j| j.jump.clone());
        team_jump(key, jump.as_deref())?;
        return config::move_jack_at(&own, &team, key);
    }
    // Leaving the team's list: nothing still in it may depend on this one.
    let theirs = patchbay::load(&team)?;
    if let Some((other, _)) = theirs
        .iter()
        .find(|(k, j)| *k != key && j.jump.as_deref() == Some(key))
    {
        return Err(format!(
            "\"{other}\" in the team list jumps through \"{key}\", so it has to stay there"
        ));
    }
    config::move_jack_at(&team, &own, key)
}

/// "Move to Team" and "Move to Mine": the table as written, to the other vault.
#[tauri::command]
pub async fn move_jack(name: String) -> Result<(), String> {
    blocking(move || {
        let from = vault_of(&name).ok_or("there is no shared list to move it to")?;
        move_between(from, &file_for(&name)?.1)
    })
    .await
}

/// Returns what was removed, for the window's Undo. The stamp is the row the window
/// showed: a device a colleague has changed since is refused, not deleted.
#[tauri::command]
pub async fn delete_jack(name: String, stamp: Option<String>) -> Result<Gone, String> {
    blocking(move || {
        let pin = load_jacks().ok().and_then(|j| rdp_pin(&j, &name).ok());
        let (file, key) = file_for(&name)?;
        let removed = config::delete_jack_at(&file, &key, stamp.as_deref())?;
        // Its remembered password goes with it, unless another device is the same desktop.
        // ponytail: an Undo brings the device back without it; it is asked for once more.
        if let Some(pin) = pin {
            let shared = load_jacks()
                .is_ok_and(|j| j.keys().any(|n| rdp_pin(&j, n).is_ok_and(|p| p == pin)));
            if !shared {
                let _ = keychain::forget(&pin);
            }
        }
        Ok(Gone {
            removed,
            vault: vault_of(&name).map(String::from),
        })
    })
    .await
}

#[tauri::command]
pub async fn restore_jack(removed: Gone) -> Result<(), String> {
    blocking(move || config::restore_jack_at(&file_of(removed.vault.as_deref())?, &removed.removed))
        .await
}

/// A drag into a folder, or "Move to…": only the folders list is written.
#[tauri::command]
pub async fn set_folders(name: String, folders: Vec<String>) -> Result<(), String> {
    blocking(move || {
        let (file, key) = file_for(&name)?;
        config::set_folders_at(&file, &key, &folders)
    })
    .await
}

/// What is written about folders, by folder path: the note, and the settings the folder
/// lends its devices. Each vault has its own tree, so each has its own.
#[tauri::command]
pub async fn notes() -> Result<Vec<Note>, String> {
    blocking(|| {
        let read = |file: PathBuf, vault| {
            let src = std::fs::read_to_string(file).unwrap_or_default();
            let mut stamps = config::stamps(&src, "folder");
            patchbay::folders(&src)
                .into_iter()
                .map(move |(path, mut f)| Note {
                    stamp: stamps.remove(&path).unwrap_or_default(),
                    note: f.note.take().unwrap_or_default().trim().to_string(),
                    defaults: f,
                    path,
                    vault,
                })
                .collect::<Vec<_>>()
        };
        if patchbay::list().is_none() {
            return Ok(read(list_file()?, None));
        }
        let mut all = read(patchbay::config_path(), Some("mine"));
        // An away list has no notes to show; the devices already say why.
        if let Ok(team) = list_file() {
            all.extend(read(team, Some("team")));
        }
        Ok(all)
    })
    .await
}

#[tauri::command]
pub async fn save_note(
    path: String,
    note: String,
    stamp: Option<String>,
    vault: Option<String>,
) -> Result<(), String> {
    blocking(move || {
        config::set_note_at(&file_of(vault.as_deref())?, &path, &note, stamp.as_deref())
    })
    .await
}

/// What a folder lends its devices - their bastion, account, key or port, written once.
#[tauri::command]
pub async fn save_folder_defaults(
    path: String,
    defaults: patchbay::Folder,
    stamp: Option<String>,
    vault: Option<String>,
) -> Result<(), String> {
    blocking(move || {
        let file = file_of(vault.as_deref())?;
        config::set_folder_defaults_at(&file, &path, &defaults, stamp.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn rename_group(
    from: String,
    to: String,
    vault: Option<String>,
) -> Result<usize, String> {
    blocking(move || config::rename_group_at(&file_of(vault.as_deref())?, &from, &to)).await
}

#[tauri::command]
pub async fn delete_group(path: String, vault: Option<String>) -> Result<usize, String> {
    blocking(move || config::delete_group_at(&file_of(vault.as_deref())?, &path)).await
}

#[tauri::command]
pub async fn snippets() -> Result<Snippets, String> {
    blocking(|| {
        let mut out = Snippets {
            list: Vec::new(),
            refused: Vec::new(),
            stamps: Default::default(),
        };
        let mut read = |file: PathBuf, vault: Option<&'static str>| {
            let src = std::fs::read_to_string(&file).unwrap_or_default();
            let (kept, refused) = patchbay::snippets(&src);
            out.list.extend(
                kept.into_iter()
                    .map(|(name, text)| Snippet { name, text, vault }),
            );
            out.refused.extend(refused);
            out.stamps
                .insert(vault.unwrap_or("team"), config::snippets_stamp_at(&file));
        };
        if patchbay::list().is_none() {
            read(list_file()?, None);
            return Ok(out);
        }
        read(patchbay::config_path(), Some("mine"));
        // An away list has none to offer; the sidebar already says why.
        if let Ok(team) = list_file() {
            read(team, Some("team"));
        }
        Ok(out)
    })
    .await
}

/// One vault's snippets, whole, as the sheet has them.
#[tauri::command]
pub async fn save_snippets(
    vault: Option<String>,
    list: Vec<(String, String)>,
    stamp: Option<String>,
) -> Result<(), String> {
    blocking(move || config::set_snippets_at(&file_of(vault.as_deref())?, &list, stamp.as_deref()))
        .await
}

/// What `~/.ssh/config` could become. Parses only: the window writes what gets ticked,
/// through `save_jack` like any other edit.
#[tauri::command]
pub fn ssh_hosts() -> Result<SshHosts, String> {
    let path = dirs::home_dir()
        .ok_or("no home directory to find an ssh config in")?
        .join(".ssh")
        .join("config");
    let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let found = import::from_ssh_config(&src);
    Ok(SshHosts {
        path: path.display().to_string(),
        hosts: found.hosts,
        warnings: found.warnings,
    })
}

/// A Royal TS document, read in the window and handed here as text, so the app never
/// opens a path it wasn't given. Parses only, like `ssh_hosts`.
#[tauri::command]
pub fn royal_hosts(src: String) -> Result<SshHosts, String> {
    let found = import::from_royal_ts(&src)?;
    Ok(SshHosts {
        path: String::new(),
        hosts: found.hosts,
        warnings: found.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_answers_for_every_target_once() {
        let a = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let b = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ports = [
            a.local_addr().unwrap().port(),
            b.local_addr().unwrap().port(),
        ];
        // More targets than the pool, the same two repeated, so the cursor has to share.
        let targets: Vec<(String, u16)> = (0..POOL + 3)
            .map(|i| ("127.0.0.1".to_string(), ports[i % 2]))
            .collect();
        let timed = probe_all(&targets);
        assert_eq!(timed.len(), 2, "one answer per distinct host:port");
        assert!(timed.values().all(Option::is_some));
    }
}
