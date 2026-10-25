//! The `#[tauri::command]` surface: everything the window can call, grouped by area.
//! Commands stay thin. Logic that is worth a test lives in the modules beside this
//! directory, most of it in `patchbay.rs`.
//!
//! The list and the config can be two files. `[settings].list` in the *own* config
//! names a shared list: the team's devices, `[defaults]` and `[folder]` notes are read
//! and written there, and the own config's `[jack.*]` are the Mine vault beside them;
//! `[settings]`, `[colors]` and everything beside the config stay on this machine. Each
//! file is parsed on its own, so nothing inherits across. In memory a device of yours
//! is `me:<key>` (`patchbay::MINE`) and the team's keeps its bare key, so both can have
//! a `web` and every command still takes one name; a `:` can't be in a link name, so
//! `patchbay://web` is always the team's. `file_for(name)` and `file_of(vault)` in
//! `commands/mod.rs` turn a name or the window's word ("mine"/"team", never a path)
//! into the file, and `load_all()` is both vaults as the one list everything reads. A
//! jump crosses one way: `merge` points a `jump` of yours at your own device when you
//! have one by that name and otherwise lets it find the team's; `via()` in the walk
//! never lets a team device find one of yours, so it reaches the same machine from
//! everyone's, whatever its file says. A move (`move_jack_at`) writes where the device
//! is going before it removes it from where it was. `list_file()` is the team's file,
//! and it is where a missing shared list is an *error* - for the team's half only:
//! `load_all` keeps it as `away`, your own devices still load, and one of them whose
//! jump may be in the missing list is refused rather than handed to ssh raw: `load`
//! reads NotFound as a first run and `read_doc` starts from the template, which on an
//! unmounted share would paint an empty list and then write a one-jack file where the
//! share mounts. Only `seed_list_at` writes at a path that isn't there, and never into
//! a directory that isn't. Every command touching the list is async: a stat on a share
//! that has gone away hangs, and a sync command hangs the main thread with it. Edits
//! carry a `stamp` (a hash of the raw table as read) and `save_jack_at`,
//! `delete_jack_at` and `set_note_at` refuse one that no longer matches; the sheet
//! takes the new stamp on refusal so the second save is a decision, not a loop. Under
//! all of that, `read_doc` hands back the bytes it parsed and `write_doc` refuses to
//! rename over a file that has moved since: every writer here replaces the whole
//! document, so a colleague's save landing in between would otherwise go with it,
//! devices the write never touched included. `list_stamp` hashes the file rather than
//! reading its mtime and size - a sync client keeps the source machine's mtime, and a
//! size doesn't move when `2222` becomes `2223`.

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
///
/// It is also where the team licence is held: once licences are on sale, a list with
/// none running is refused here like one that is away, so every read and write of it
/// stops in one place and your own devices still load. `licence_list` is the way past,
/// for the licence itself.
pub fn list_file() -> Result<PathBuf, String> {
    let Some(p) = licence_list()? else {
        return Ok(patchbay::config_path());
    };
    // Not even read until licences are on sale: this runs on every touch of the list,
    // over a share, and the answer could only be yes.
    if !crate::licence::ON_SALE {
        return Ok(p);
    }
    let standing = crate::licence::status(&p);
    if !crate::licence::opens(&standing) {
        return Err(crate::licence::refusal(&standing));
    }
    Ok(p)
}

/// The shared list whatever its licence, or None with one vault: where a licence is
/// read, added and removed.
pub fn licence_list() -> Result<Option<PathBuf>, String> {
    let Some(p) = patchbay::list() else {
        return Ok(None);
    };
    if p.is_file() {
        Ok(Some(p))
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
