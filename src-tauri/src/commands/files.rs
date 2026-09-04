//! The file browser. Every call is its own `sftp` run sharing one ssh session through
//! multiplexing; see `sftp.rs`.

use super::os_open;
use crate::sftp;
use serde::Serialize;
use std::path::Path;

#[tauri::command]
pub async fn sftp_ls(name: String, path: String) -> Result<sftp::Listing, String> {
    super::blocking(move || sftp::ls(&name, &path)).await
}

/// Polled by a files tab waiting on a shell to authenticate. A `stat`, not a
/// connection: asking by trying would be a failed login every second.
#[tauri::command]
pub fn sftp_ready(name: String) -> Result<bool, String> {
    sftp::ready(&name)
}

/// Returns where the download landed, so the window can say which folder.
#[tauri::command]
pub async fn sftp_get(name: String, remote: String, recurse: bool) -> Result<String, String> {
    super::blocking(move || {
        sftp::get(&name, &remote, &sftp::downloads(), recurse).map(|p| p.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn sftp_put(name: String, local: String, remote_dir: String) -> Result<(), String> {
    super::blocking(move || sftp::put(&name, Path::new(&local), &remote_dir)).await
}

/// Make, rename or remove.
#[tauri::command]
pub async fn sftp_edit(name: String, op: String, path: String, to: String) -> Result<(), String> {
    super::blocking(move || sftp::edit(&name, &op, &path, &to)).await
}

/// Emitted each time an opened file is saved back.
#[derive(Clone, Serialize)]
struct SavedBack {
    name: String,
    file: String,
    error: Option<String>,
}

/// "Edit here": download, open with the desktop's default app, and upload on every
/// save. The far end knows nothing about a file being open, so the copy's mtime is
/// the only signal there is. Returns where the copy is.
#[tauri::command]
pub async fn sftp_open(
    app: tauri::AppHandle,
    name: String,
    remote: String,
) -> Result<String, String> {
    super::blocking(move || {
        let dir = remote.rsplit_once('/').map_or(".", |(d, _)| d).to_string();
        let local = sftp::get(&name, &remote, &sftp::edit_dir(&name), false)?;
        let seen = modified(&local);
        os_open(local.as_os_str())?;
        let at = local.display().to_string();
        // ponytail: one polling thread per opened file; a platform watcher is the
        // upgrade if anyone opens dozens.
        std::thread::spawn(move || watch_edit(&app, &name, &local, &dir, seen));
        Ok(at)
    })
    .await
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Upload the copy whenever its mtime changes. Ends when the copy is gone or an
/// upload is refused, rather than retrying against a host that just said no.
fn watch_edit(
    app: &tauri::AppHandle,
    name: &str,
    local: &Path,
    dir: &str,
    mut seen: Option<std::time::SystemTime>,
) {
    use tauri::Emitter;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let Some(now) = modified(local) else { return };
        if Some(now) == seen {
            continue;
        }
        seen = Some(now);
        let error = sftp::put(name, local, dir).err();
        let failed = error.is_some();
        let _ = app.emit(
            "sftp:saved",
            SavedBack {
                name: name.to_string(),
                file: local
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                error,
            },
        );
        if failed {
            return;
        }
    }
}

/// The Full Disk Access panel: macOS hands `sftp-server` an empty Desktop, Documents
/// or Downloads until the app is listed there. A fixed url, so nothing from a config
/// reaches the opener.
#[tauri::command]
pub fn open_full_disk_access() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return os_open(std::ffi::OsStr::new(
        "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
    ));
    #[cfg(not(target_os = "macos"))]
    Err("that setting is a macOS one".into())
}
