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

/// TCP-connect every device's entry point in parallel. Nothing is sent.
/// ponytail: one thread per jack, bounded pool if someone brings a thousand.
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

        Ok(std::thread::scope(|s| {
            let handles: Vec<_> = targets
                .iter()
                .map(|(name, host, port)| {
                    s.spawn(move || Probe {
                        name: name.clone(),
                        target: format!("{host}:{port}"),
                        ms: connect_ms(host, *port),
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        }))
    })
    .await
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

#[tauri::command]
pub fn delete_jack(name: String) -> Result<(), String> {
    config::delete_jack_at(&patchbay::config_path(), &name)
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
