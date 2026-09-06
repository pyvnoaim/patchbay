//! The file browser. Every call is its own `sftp` run sharing one ssh session through
//! multiplexing; see `sftp.rs`.

use super::{blocking, os_open};
use crate::sftp;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[tauri::command]
pub async fn sftp_ls(name: String, path: String) -> Result<sftp::Listing, String> {
    blocking(move || sftp::ls(&name, &path)).await
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
    blocking(move || {
        sftp::get(&name, &remote, &sftp::downloads(), recurse).map(|p| p.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn sftp_put(name: String, local: String, remote_dir: String) -> Result<(), String> {
    blocking(move || sftp::put(&name, Path::new(&local), &remote_dir)).await
}

/// Make, rename or remove.
#[tauri::command]
pub async fn sftp_edit(name: String, op: String, path: String, to: String) -> Result<(), String> {
    blocking(move || sftp::edit(&name, &op, &path, &to)).await
}

/// Emitted each time an opened file is saved back.
#[derive(Clone, Serialize)]
struct SavedBack {
    name: String,
    file: String,
    error: Option<String>,
}

/// A copy open for "Edit here": which host it came from, where it goes back to, and the
/// mtime last uploaded, so a save that changes nothing isn't sent twice.
struct Edit {
    name: String,
    dir: String,
    seen: Option<std::time::SystemTime>,
}

type Open = Arc<Mutex<HashMap<PathBuf, Edit>>>;

/// Every open copy, and the one OS watcher over their folders. Managed by Tauri, so it
/// lives as long as the app and one watcher serves every file.
#[derive(Default)]
pub struct Edits {
    open: Open,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    dirs: Mutex<HashSet<PathBuf>>,
}

impl Edits {
    /// Register a copy and make sure its folder is watched. The folder, not the file:
    /// most editors save by writing a temp file and renaming it over the old one, and a
    /// watch on the file itself follows the old inode into the bin.
    fn watch(
        &self,
        app: &tauri::AppHandle,
        local: PathBuf,
        name: &str,
        dir: &str,
    ) -> Result<(), String> {
        use notify::Watcher;
        // Canonical, because that is how the OS names it back: macOS's temp dir is
        // `/var/...`, and FSEvents reports `/private/var/...`.
        let local = local.canonicalize().unwrap_or(local);
        let seen = modified(&local);
        self.open.lock().unwrap().insert(
            local.clone(),
            Edit {
                name: name.to_string(),
                dir: dir.to_string(),
                seen,
            },
        );
        let mut watcher = self.watcher.lock().unwrap();
        if watcher.is_none() {
            let (open, app) = (self.open.clone(), app.clone());
            let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(e) = res {
                    on_event(&app, &open, &e.paths);
                }
            })
            .map_err(|e| format!("could not watch {}: {e}", local.display()))?;
            *watcher = Some(w);
        }
        let folder = local.parent().unwrap_or(&local).to_path_buf();
        if self.dirs.lock().unwrap().insert(folder.clone()) {
            watcher
                .as_mut()
                .unwrap()
                .watch(&folder, notify::RecursiveMode::NonRecursive)
                .map_err(|e| format!("could not watch {}: {e}", folder.display()))?;
        }
        Ok(())
    }
}

/// "Edit here": download, open with the desktop's default app, and upload on every
/// save. The far end knows nothing about a file being open, so the copy changing on
/// disk is the only signal there is. Returns where the copy is.
#[tauri::command]
pub async fn sftp_open(
    app: tauri::AppHandle,
    edits: tauri::State<'_, Edits>,
    name: String,
    remote: String,
) -> Result<String, String> {
    let dir = remote_dir(&remote);
    let local = blocking({
        let name = name.clone();
        move || sftp::get(&name, &remote, &sftp::edit_dir(&name), false)
    })
    .await?;
    edits.watch(&app, local.clone(), &name, &dir)?;
    os_open(local.as_os_str())?;
    Ok(local.display().to_string())
}

fn remote_dir(remote: &str) -> String {
    remote.rsplit_once('/').map_or(".", |(d, _)| d).to_string()
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// The mtime once it has stopped moving, or None if the file went away. Gives up
/// waiting after two seconds and takes what is there.
fn settled(p: &Path, mut last: std::time::SystemTime) -> Option<std::time::SystemTime> {
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let now = modified(p)?;
        if now == last {
            return Some(now);
        }
        last = now;
    }
    Some(last)
}

/// A change under a watched folder: upload every open copy it names whose mtime moved.
/// A folder path (the watcher saying "look again") checks everything in that folder.
/// Ends a copy when it is gone or an upload is refused, rather than retrying against a
/// host that just said no. Uploads run here, on the watcher's thread, one at a time.
fn on_event(app: &tauri::AppHandle, open: &Open, paths: &[PathBuf]) {
    use tauri::Emitter;
    let hit: Vec<PathBuf> = {
        let open = open.lock().unwrap();
        open.keys()
            .filter(|k| {
                paths
                    .iter()
                    .any(|p| p == *k || Some(p.as_path()) == k.parent())
            })
            .cloned()
            .collect()
    };
    for local in hit {
        let Some(now) = modified(&local) else {
            open.lock().unwrap().remove(&local);
            continue;
        };
        let (name, dir) = {
            let open = open.lock().unwrap();
            let Some(e) = open.get(&local) else { continue };
            if e.seen == Some(now) {
                continue;
            }
            (e.name.clone(), e.dir.clone())
        };
        // An editor writing in place fires the first event mid-write. Wait for the mtime
        // to hold still before reading the file, or half of it goes up.
        let Some(now) = settled(&local, now) else {
            open.lock().unwrap().remove(&local);
            continue;
        };
        if let Some(e) = open.lock().unwrap().get_mut(&local) {
            e.seen = Some(now);
        }
        let error = sftp::put(&name, &local, &dir).err();
        if error.is_some() {
            open.lock().unwrap().remove(&local);
        }
        let _ = app.emit(
            "sftp:saved",
            SavedBack {
                name,
                file: local
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                error,
            },
        );
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
