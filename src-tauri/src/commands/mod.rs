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

/// Where the team's list is read and written: the shared list, or the own config when
/// there is none. A shared list that is not there is an error, never an empty list and
/// never a fresh file: a share that is away must not read as empty, and nothing is
/// written where it will mount. The own config keeps the first-run rule, where missing
/// means empty.
pub fn list_file() -> Result<PathBuf, String> {
    let Some(p) = patchbay::list() else {
        return Ok(patchbay::config_path());
    };
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("the team list at {} is not there", p.display()))
    }
}

/// The file a vault is, by the word the window sends: "mine" is this machine's own
/// config, anything else the team's. Never a path from the window.
pub fn file_of(vault: Option<&str>) -> Result<PathBuf, String> {
    match vault {
        Some("mine") => Ok(patchbay::config_path()),
        _ => list_file(),
    }
}

/// The file a device is in and the key it has there. With a shared list set, a name
/// from your own vault arrives as `me:<key>`; with none there is one file and one name.
pub fn file_for(name: &str) -> Result<(PathBuf, String), String> {
    match name.strip_prefix(patchbay::MINE) {
        Some(key) if patchbay::list().is_some() => Ok((patchbay::config_path(), key.into())),
        _ => Ok((list_file()?, name.into())),
    }
}

/// Both vaults as the one list everything reads. `away` is why the team's half is
/// missing, when it is: your own devices still load, and the window says so above them.
pub struct Loaded {
    pub jacks: patchbay::Jacks,
    pub sources: patchbay::Sources,
    pub away: Option<String>,
}

pub fn load_all() -> Result<Loaded, String> {
    let own = patchbay::config_path();
    if patchbay::list().is_none() {
        let (jacks, sources) = patchbay::load_all(&own)?;
        return Ok(Loaded {
            jacks,
            sources,
            away: None,
        });
    }
    let (mine, said) = patchbay::load_all(&own)?;
    let mut sources: patchbay::Sources = said
        .into_iter()
        .map(|(k, v)| (format!("{}{k}", patchbay::MINE), v))
        .collect();
    let (team, away) = match list_file().and_then(|p| patchbay::load_all(&p)) {
        Ok((jacks, theirs)) => {
            sources.extend(theirs);
            (Some(jacks), None)
        }
        Err(e) => (None, Some(e)),
    };
    Ok(Loaded {
        jacks: patchbay::merge(team, mine),
        sources,
        away,
    })
}

pub fn load_jacks() -> Result<patchbay::Jacks, String> {
    load_all().map(|l| l.jacks)
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
