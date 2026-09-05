//! `[settings]`, `[defaults]`, `[colors]`, the theme, and the config file itself.

use super::{blocking, list_file, os_open, ssh_dir};
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

/// `[defaults]` is part of the list: a shared `user` there is everyone's.
#[tauri::command]
pub async fn defaults() -> Result<config::Defaults, String> {
    blocking(|| Ok(config::load_defaults_at(&list_file()?))).await
}

#[tauri::command]
pub async fn save_defaults(next: config::Defaults) -> Result<(), String> {
    blocking(move || config::save_defaults_at(&list_file()?, &next)).await
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
