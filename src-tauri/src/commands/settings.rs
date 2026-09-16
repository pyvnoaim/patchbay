//! `[settings]`, `[defaults]`, `[colors]`, the theme, and the config file itself.

use super::{blocking, list_file, os_open, ssh_dir, writable_list};
use crate::{config, patchbay};

#[tauri::command]
pub fn settings() -> config::Settings {
    config::load_settings()
}

#[tauri::command]
pub async fn save_settings(next: config::Settings) -> Result<(), String> {
    blocking(move || {
        let was = config::load_settings();
        // Pointing at a list that isn't there yet seeds it with this machine's, before
        // the setting is written: a seed that fails leaves the setting as it was. Only
        // for a path just typed: a share that is away must not block the theme.
        if let Some(l) = next
            .list
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty() && next.list != was.list)
        {
            let path = std::path::PathBuf::from(patchbay::expand(l));
            if !path.is_absolute() {
                return Err(format!("\"{l}\" isn't a full path to a file"));
            }
            if !path.exists() {
                config::seed_list_at(&patchbay::config_path(), &path)?;
            } else if !path.is_file() {
                return Err(format!("{} is a folder, not a file", path.display()));
            }
        }
        // `jacks` only ever writes the ssh include, so turning the switch off has to
        // remove it here.
        config::save_settings(&next)?;
        if was.write_ssh_config && !next.write_ssh_config {
            if let Some(dir) = ssh_dir() {
                config::remove_ssh_include(&dir)?;
            }
        }
        Ok(())
    })
    .await
}

/// `[defaults]` is part of the list: a shared `user` there is everyone's, so it carries
/// a stamp the way a device does.
#[derive(serde::Serialize)]
pub struct DefaultsView {
    #[serde(flatten)]
    defaults: config::Defaults,
    stamp: String,
}

#[tauri::command]
pub async fn defaults() -> Result<DefaultsView, String> {
    blocking(|| {
        let path = list_file()?;
        Ok(DefaultsView {
            defaults: config::load_defaults_at(&path),
            stamp: config::defaults_stamp_at(&path),
        })
    })
    .await
}

#[tauri::command]
pub async fn save_defaults(next: config::Defaults, stamp: Option<String>) -> Result<(), String> {
    blocking(move || config::save_defaults_at(&writable_list()?, &next, stamp.as_deref())).await
}

#[tauri::command]
pub fn colors() -> std::collections::BTreeMap<String, String> {
    config::load_colors()
}

#[tauri::command]
pub fn save_color(os: String, hex: Option<String>) -> Result<(), String> {
    config::save_color(&os, hex.as_deref())
}

/// Pin the window's appearance and answer with what it is actually wearing. The page
/// can't tell on its own: vibrancy and the title bar follow the window, not our CSS,
/// and our `color-scheme` makes `prefers-color-scheme` echo the last pin back.
#[tauri::command]
pub fn set_theme(window: tauri::WebviewWindow, theme: String) -> String {
    let want = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    let _ = window.set_theme(want);
    let pick = match want.or_else(|| window.theme().ok()) {
        Some(tauri::Theme::Light) => "light",
        _ => "dark",
    };
    #[cfg(windows)]
    paint_caption(&window, pick == "dark");
    pick.into()
}

/// Windows paints the title bar its own dark, a few shades off ours, so the window reads
/// as two slabs stacked. DWM takes a COLORREF, which is `0x00bbggrr` rather than a hex
/// colour; both values are `--bg` in `app.css`. Win11 22000 and up - an older build fails
/// the call and keeps the default title bar, which is what it had anyway.
#[cfg(windows)]
fn paint_caption(window: &tauri::WebviewWindow, dark: bool) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR,
    };
    let Ok(hwnd) = window.hwnd() else { return };
    let bg: u32 = if dark { 0x001a_1717 } else { 0x00f6_f4f4 };
    let p = std::ptr::addr_of!(bg).cast();
    let n = std::mem::size_of::<u32>() as u32;
    // SAFETY: a live window's handle, and DWM reads `n` bytes out of a `u32` that outlives the call.
    unsafe {
        DwmSetWindowAttribute(hwnd.0 as _, DWMWA_CAPTION_COLOR as u32, p, n);
        DwmSetWindowAttribute(hwnd.0 as _, DWMWA_BORDER_COLOR as u32, p, n);
    }
}

/// Private keys in `~/.ssh`, for the key field to suggest. Returned `~`-prefixed so a
/// path written into a shared config still works on a colleague's machine.
#[tauri::command]
pub fn ssh_keys() -> Vec<String> {
    match dirs::home_dir() {
        Some(h) => keys_in(&h.join(".ssh")),
        None => vec![],
    }
}

/// A key is recognised by its `.pub` sibling, which rules out `config` and
/// `known_hosts` without naming them.
fn keys_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut keys: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let paired = e.path().with_file_name(format!("{name}.pub")).is_file();
            (!name.ends_with(".pub") && paired).then(|| format!("~/.ssh/{name}"))
        })
        .collect();
    keys.sort();
    keys
}

/// What a previous install left in `~/.ssh`. Only reported while the setting is off:
/// with it on, the file is the feature working.
#[tauri::command]
pub fn ssh_leftovers() -> Option<config::Leftovers> {
    if config::load_settings().write_ssh_config {
        return None;
    }
    let dir = ssh_dir()?;
    let l = config::ssh_leftovers(&dir);
    (l.conf_file || l.include_line).then_some(l)
}

#[tauri::command]
pub fn clean_ssh_leftovers() -> Result<(), String> {
    let dir = ssh_dir().ok_or("no ~/.ssh on this machine")?;
    config::remove_ssh_include(&dir)
}

#[derive(serde::Serialize)]
pub struct Paths {
    /// Where the devices are: the shared list, or the own config.
    list: String,
    /// This machine's config, which is where `[settings]` and `[colors]` always are.
    own: String,
}

#[tauri::command]
pub fn config_path() -> Paths {
    Paths {
        list: patchbay::list_path().display().to_string(),
        own: patchbay::config_path().display().to_string(),
    }
}

/// The OS picker, for naming a shared list without typing a share's path. Whoever
/// starts the list has no file to pick yet, so `folder` asks for a folder and answers
/// `patchbay.toml` in it, which `save_settings` seeds; everyone after picks the file.
/// No panel here takes both, and a save panel would ask the next person whether to
/// "replace" a list that is only going to be read. Returns `None` when the panel is
/// cancelled, which is an answer rather than a failure.
///
/// The blocking pickers must not run on the main thread, and a command doesn't: it is
/// the runtime's, which is exactly what this needs.
#[tauri::command]
pub async fn pick_list_file(app: tauri::AppHandle, folder: bool) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt as _;
    blocking(move || {
        let panel = app.dialog().file();
        let picked = if folder {
            panel
                .set_title("Choose a folder for the new list")
                .blocking_pick_folder()
                .and_then(|p| p.into_path().ok())
                .map(|p| p.join("patchbay.toml"))
        } else {
            panel
                .add_filter("patchbay list", &["toml"])
                .set_title("Choose a shared list")
                .blocking_pick_file()
                .and_then(|p| p.into_path().ok())
        };
        Ok(picked.map(|p| p.display().to_string()))
    })
    .await
}

/// Opens the list, which is what "the config" means to someone editing devices by
/// hand. Only the own config is created on the way: a missing shared list is an error.
#[tauri::command]
pub async fn open_config() -> Result<(), String> {
    blocking(|| {
        let p = list_file()?;
        config::ensure_exists(&p)?;
        os_open(p.as_os_str())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::keys_in;

    #[test]
    fn only_private_keys_with_a_public_half_are_suggested() {
        let dir = std::env::temp_dir().join(format!("patchbay-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "id_ed25519",
            "id_ed25519.pub",
            "known_hosts",
            "config",
            "work.key",
            "work.key.pub",
        ] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        assert_eq!(keys_in(&dir), ["~/.ssh/id_ed25519", "~/.ssh/work.key"]);
    }
}
