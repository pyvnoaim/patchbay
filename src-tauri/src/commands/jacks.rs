//! The device list: reading it for the window, editing it, folders and notes, and
//! importing one from an ssh config or a Royal TS document.

use super::{blocking, list_file, load_jacks, os_open, ssh_dir, writable_list};
use crate::{config, import, patchbay, terminal};
use serde::Serialize;

#[derive(Serialize)]
pub struct JackView {
    name: String,
    host: String,
    user: Option<String>,
    port: Option<u16>,
    key: Option<String>,
    jump: Option<String>,
    os: Option<String>,
    url: Option<String>,
    rdp: Option<u16>,
    vnc: Option<u16>,
    ssh: bool,
    primary: String,
    desc: Option<String>,
    folders: Vec<String>,
    forward: Vec<String>,
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

#[derive(Serialize)]
pub struct Note {
    path: String,
    note: String,
    stamp: String,
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
    if !config::load_settings().write_ssh_config {
        return;
    }
    let Some(dir) = ssh_dir() else { return };
    let theirs = std::fs::read_to_string(dir.join("config"))
        .map(|s| import::host_names(&s))
        .unwrap_or_default();
    let _ = config::write_ssh_include(&dir, &import::to_ssh_config(jacks, &theirs));
}

#[tauri::command]
pub async fn jacks() -> Result<Vec<JackView>, String> {
    blocking(|| {
        let path = list_file()?;
        let jacks = patchbay::load(&path)?;
        sync_ssh_config(&jacks);
        let mut stamps = config::stamps_at(&path, "jack");
        Ok(jacks
            .iter()
            .map(|(name, j)| JackView {
                stamp: stamps.remove(name),
                name: name.clone(),
                host: j.host.clone(),
                user: j.user.clone(),
                port: j.port,
                jump: j.jump.clone(),
                os: j.os.clone(),
                url: j.url.clone(),
                rdp: j.rdp,
                vnc: j.vnc,
                ssh: j.ssh.unwrap_or(true),
                primary: patchbay::primary(j),
                desc: j.desc.clone(),
                key: j.key.clone(),
                folders: j.folders.clone().unwrap_or_default(),
                forward: j.forward.clone().unwrap_or_default(),
                hops: patchbay::hops(name, &jacks).unwrap_or_default(),
                command: patchbay::ssh_args(name, &jacks)
                    .map(|a| terminal::command_line(&a))
                    .unwrap_or_else(|e| e),
            })
            .collect())
    })
    .await
}

/// Cheap enough for a timer: one read of a file that is kilobytes, and one directory
/// listing when the list is shared. Dropbox writes `x (conflicted copy ...)`, OneDrive
/// `x-MACHINE`; anything else with the list's stem and extension in that directory is
/// close enough to name.
#[tauri::command]
pub async fn list_stamp() -> Result<ListStamp, String> {
    blocking(|| {
        use std::hash::{Hash, Hasher};
        let path = list_file()?;
        let src = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut h = std::hash::DefaultHasher::new();
        src.hash(&mut h);
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

#[tauri::command]
pub async fn save_jack(original: Option<String>, jack: config::JackInput) -> Result<(), String> {
    blocking(move || config::save_jack_at(&writable_list()?, original, jack)).await
}

/// Returns what was removed, for the window's Undo. The stamp is the row the window
/// showed: a device a colleague has changed since is refused, not deleted.
#[tauri::command]
pub async fn delete_jack(name: String, stamp: Option<String>) -> Result<config::Removed, String> {
    blocking(move || config::delete_jack_at(&writable_list()?, &name, stamp.as_deref())).await
}

#[tauri::command]
pub async fn restore_jack(removed: config::Removed) -> Result<(), String> {
    blocking(move || config::restore_jack_at(&writable_list()?, &removed)).await
}

/// A drag into a folder, or "Move to…": only the folders list is written.
#[tauri::command]
pub async fn set_folders(name: String, folders: Vec<String>) -> Result<(), String> {
    blocking(move || config::set_folders_at(&writable_list()?, &name, &folders)).await
}

/// The notes hung on folders, by folder path.
#[tauri::command]
pub async fn notes() -> Result<Vec<Note>, String> {
    blocking(|| {
        let src = std::fs::read_to_string(list_file()?).unwrap_or_default();
        let mut stamps = config::stamps(&src, "folder");
        Ok(patchbay::notes(&src)
            .into_iter()
            .map(|(path, note)| Note {
                stamp: stamps.remove(&path).unwrap_or_default(),
                path,
                note,
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn save_note(path: String, note: String, stamp: Option<String>) -> Result<(), String> {
    blocking(move || config::set_note_at(&writable_list()?, &path, &note, stamp.as_deref())).await
}

#[tauri::command]
pub async fn rename_group(from: String, to: String) -> Result<usize, String> {
    blocking(move || config::rename_group_at(&writable_list()?, &from, &to)).await
}

#[tauri::command]
pub async fn delete_group(path: String) -> Result<usize, String> {
    blocking(move || config::delete_group_at(&writable_list()?, &path)).await
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
