//! `[settings]`, `[defaults]`, `[colors]`, the theme, and the config file itself.

use super::{os_open, ssh_dir};
use crate::{config, patchbay};

#[tauri::command]
pub fn settings() -> config::Settings {
    config::load_settings()
}

#[tauri::command]
pub fn save_settings(next: config::Settings) -> Result<(), String> {
    // `jacks` only ever writes the ssh include, so turning the switch off has to
    // remove it here.
    let was = config::load_settings().write_ssh_config;
    config::save_settings(&next)?;
    if was && !next.write_ssh_config {
        if let Some(dir) = ssh_dir() {
            config::remove_ssh_include(&dir)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn defaults() -> config::Defaults {
    config::load_defaults()
}

#[tauri::command]
pub fn save_defaults(next: config::Defaults) -> Result<(), String> {
    config::save_defaults(&next)
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
    match want.or_else(|| window.theme().ok()) {
        Some(tauri::Theme::Light) => "light".into(),
        _ => "dark".into(),
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

#[tauri::command]
pub fn config_path() -> String {
    patchbay::config_path().display().to_string()
}

#[tauri::command]
pub fn open_config() -> Result<(), String> {
    let p = patchbay::config_path();
    config::ensure_exists(&p)?;
    os_open(p.as_os_str())
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
