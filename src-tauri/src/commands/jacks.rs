//! The device list: reading it for the window, editing it, folders and notes, and
//! importing one from an ssh config or a Royal TS document.

use super::{load_jacks, ssh_dir};
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
pub fn jacks() -> Result<Vec<JackView>, String> {
    let jacks = load_jacks()?;
    sync_ssh_config(&jacks);
    Ok(jacks
        .iter()
        .map(|(name, j)| JackView {
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
}

/// TCP-connect every device's entry point in parallel. Nothing is sent. Each distinct
/// host:port is dialled once - twenty devices behind one bastion are one connection -
/// and at most `POOL` at a time, so a thousand devices don't mean a thousand threads.
#[tauri::command]
pub async fn probe() -> Result<Vec<Probe>, String> {
    super::blocking(|| {
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

#[tauri::command]
pub fn save_jack(original: Option<String>, jack: config::JackInput) -> Result<(), String> {
    config::save_jack_at(&patchbay::config_path(), original, jack)
}

/// Returns what was removed, for the window's Undo.
#[tauri::command]
pub fn delete_jack(name: String) -> Result<config::Removed, String> {
    config::delete_jack_at(&patchbay::config_path(), &name)
}

#[tauri::command]
pub fn restore_jack(removed: config::Removed) -> Result<(), String> {
    config::restore_jack_at(&patchbay::config_path(), &removed)
}

/// A drag into a folder, or "Move to…": only the folders list is written.
#[tauri::command]
pub fn set_folders(name: String, folders: Vec<String>) -> Result<(), String> {
    config::set_folders_at(&patchbay::config_path(), &name, &folders)
}

/// The notes hung on folders, by folder path.
#[tauri::command]
pub fn notes() -> Vec<Note> {
    let src = std::fs::read_to_string(patchbay::config_path()).unwrap_or_default();
    patchbay::notes(&src)
        .into_iter()
        .map(|(path, note)| Note { path, note })
        .collect()
}

#[tauri::command]
pub fn save_note(path: String, note: String) -> Result<(), String> {
    config::set_note_at(&patchbay::config_path(), &path, &note)
}

#[tauri::command]
pub fn rename_group(from: String, to: String) -> Result<usize, String> {
    config::rename_group_at(&patchbay::config_path(), &from, &to)
}

#[tauri::command]
pub fn delete_group(path: String) -> Result<usize, String> {
    config::delete_group_at(&patchbay::config_path(), &path)
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
