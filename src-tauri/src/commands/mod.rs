//! The `#[tauri::command]` surface: everything the window can call, grouped by area.
//! Commands stay thin. Logic that is worth a test lives in the modules beside this
//! directory, most of it in `patchbay.rs`.

pub mod app;
pub mod files;
pub mod jacks;
pub mod remote;
pub mod sessions;
pub mod settings;
pub mod web;

use crate::patchbay;
use std::path::PathBuf;

/// The device list from this machine's config. A missing file is an empty list.
pub fn load_jacks() -> Result<patchbay::Jacks, String> {
    patchbay::load(&patchbay::config_path())
}

/// Where ssh keeps its own config, and where ours goes beside it.
pub fn ssh_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh"))
}

/// Hand a path or URL to the desktop's default handler. Never through a shell:
/// `cmd /c start` would read an `&` in a URL as a command separator.
pub fn os_open(arg: &std::ffi::OsStr) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(arg)
        .spawn()
        .map_err(|e| format!("could not open {}: {e}", arg.to_string_lossy()))?;
    Ok(())
}

/// Run blocking work off the async runtime and flatten the join error into ours.
pub async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}
