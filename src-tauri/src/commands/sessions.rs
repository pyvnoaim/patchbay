//! Terminal sessions: ssh on a pty inside the window, ssh in the system terminal, and
//! the one-shot ping / traceroute tabs.

use super::load_jacks;
use crate::{config, patchbay, pty, sftp, terminal};

/// A device name, or a `user@host` that isn't one. The device error is the one kept
/// when neither reads: "no jack matching" is what a typo gets.
fn ssh_target(
    name: &str,
    jacks: &patchbay::Jacks,
) -> Result<(Option<String>, Vec<String>), String> {
    match patchbay::resolve(name, jacks) {
        Ok(resolved) => {
            let args = patchbay::ssh_args(&resolved, jacks)?;
            Ok((Some(resolved), args))
        }
        Err(e) => patchbay::adhoc_args(name).map(|a| (None, a)).map_err(|_| e),
    }
}

/// Open the device in the system terminal.
#[tauri::command]
pub fn connect(name: String) -> Result<String, String> {
    let jacks = load_jacks()?;
    let (_, args) = ssh_target(&name, &jacks)?;
    terminal::open(&args)?;
    Ok(terminal::command_line(&args))
}

/// Open the device as a tab in the window.
#[tauri::command]
pub fn open_session(
    app: tauri::AppHandle,
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let jacks = load_jacks()?;
    let (resolved, args) = ssh_target(&name, &jacks)?;
    // The shell doubles as the connection the file browser rides: `sftp -b` cannot ask
    // for a password, so a session here is what authenticates it. A quick connect has
    // no device for the browser to name, so it shares nothing.
    let mux = resolved
        .as_deref()
        .map(|r| sftp::mux(&sftp::control_path(r)))
        .unwrap_or_default();
    let spawned: Vec<String> = mux.into_iter().chain(args.iter().cloned()).collect();
    let log = config::load_settings()
        .log_sessions
        .then(|| log_path(resolved.as_deref().unwrap_or(&name)));
    sessions.open(&app, id, "ssh", &spawned, cols.max(2), rows.max(2), log)?;
    Ok(terminal::command_line(&args))
}

/// A device name as a file name: anything that could be a path separator is replaced,
/// so a hand-edited name can't land the log somewhere else.
fn log_path(name: &str) -> std::path::PathBuf {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    patchbay::logs_dir().join(format!("{safe}-{stamp}.log"))
}

/// Ping or traceroute in a tab, on the same pty a session uses so output streams and
/// ^C works. The tab goes dead when the command exits.
#[tauri::command]
pub fn open_task(
    app: tauri::AppHandle,
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    name: String,
    task: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let jacks = load_jacks()?;
    let resolved = patchbay::resolve(&name, &jacks)?;
    let (program, args) = patchbay::task_argv(&task, &resolved, &jacks)?;
    sessions.open(&app, id, &program, &args, cols.max(2), rows.max(2), None)?;
    Ok(terminal::command_line_of(&program, &args))
}

/// `ssh -N` on a pty, so a files tab can authenticate when no shell has: a password or
/// host-key question is answered in the tab itself. Not on Windows, where ssh has no
/// connection multiplexing to share.
#[tauri::command]
pub fn open_master(
    app: tauri::AppHandle,
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = (app, sessions, id, name, cols, rows);
        return Err(
            "ssh on Windows can't share a connection, so files need a key or your agent".into(),
        );
    }
    #[cfg(not(windows))]
    {
        let jacks = load_jacks()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let args = patchbay::without_forwards(patchbay::ssh_args(&resolved, &jacks)?);
        // -N first: everything after the destination would be a remote command.
        let mut spawned = vec!["-N".to_string()];
        spawned.extend(sftp::mux(&sftp::control_path(&resolved)));
        spawned.extend(args);
        sessions.open(&app, id, "ssh", &spawned, cols.max(2), rows.max(2), None)
    }
}

#[tauri::command]
pub fn write_session(
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    data: String,
) -> Result<(), String> {
    sessions.write(id, &data)
}

#[tauri::command]
pub fn resize_session(
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    sessions.resize(id, cols.max(2), rows.max(2))
}

#[tauri::command]
pub fn close_session(sessions: tauri::State<'_, pty::Shared>, id: u32) {
    sessions.close(id);
}
