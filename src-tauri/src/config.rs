//! Writing the config back out. The file is something people hand-edit, so every
//! change goes through toml_edit — comments, spacing and key order survive — and
//! lands via a temp file + rename so a crash mid-write can't truncate it.

use crate::patchbay;
use serde::{Deserialize, Serialize};
use std::path::Path;
use toml_edit::{value, Array, DocumentMut, Item, Table};

#[derive(Debug, Deserialize)]
pub struct JackInput {
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key: Option<String>,
    pub jump: Option<String>,
    pub os: Option<String>,
    pub url: Option<String>,
    pub rdp: Option<u16>,
    pub ssh: Option<bool>,
    pub primary: Option<String>,
    pub desc: Option<String>,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub forward: Vec<String>,
}

fn read_doc(path: &Path) -> Result<DocumentMut, String> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    src.parse::<DocumentMut>()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Same directory, so the rename is atomic — the config is never half-written.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// toml_edit hangs the lines above a table on that table, so removing one deletes
/// the comments sitting over it — including a file header that was never about it.
/// Hands them back for `rehome_comments` instead.
/// ponytail: the whole block moves, so a comment about a deleted jack ends up above
/// the next one. A stale comment is visible and fixable; a deleted one isn't.
pub fn orphan_comments(parent: &mut Table, key: &str) -> Option<(String, usize)> {
    let removed = parent.remove(key)?;
    let t = removed.as_table()?;
    let prefix = t.decor().prefix()?.as_str()?;
    if prefix.trim().is_empty() {
        return None;
    }
    Some((prefix.to_string(), t.position()?))
}

/// Tables render in `position()` order, so the one that takes the removed table's
/// place is the next position along — wherever in the tree it happens to live.
fn first_position_after(item: &Item, after: usize) -> Option<usize> {
    let t = item.as_table()?;
    t.iter()
        .filter_map(|(_, v)| first_position_after(v, after))
        .chain(t.position().filter(|p| *p > after))
        .min()
}

fn prepend_prefix(item: &mut Item, at: usize, comments: &str) -> bool {
    let Some(t) = item.as_table_mut() else { return false };
    if t.position() == Some(at) {
        let old = t.decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
        t.decor_mut().set_prefix(format!("{comments}{old}"));
        return true;
    }
    t.iter_mut().any(|(_, v)| prepend_prefix(v, at, comments))
}

pub fn rehome_comments(doc: &mut DocumentMut, orphan: Option<(String, usize)>) {
    let Some((comments, was_at)) = orphan else { return };
    match first_position_after(doc.as_item(), was_at) {
        Some(at) => {
            prepend_prefix(doc.as_item_mut(), at, &comments);
        }
        // Nothing renders after it, so the file ends with them.
        None => {
            let trailing = doc.trailing().as_str().unwrap_or("").to_string();
            doc.set_trailing(format!("{comments}{trailing}"));
        }
    }
}

/// `[jack]` is implicit — we only ever write the `[jack.name]` children.
fn jack_table(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    let item = doc.entry("jack").or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(true);
        Item::Table(t)
    });
    let t = item
        .as_table_mut()
        .ok_or("`jack` in the config isn't a table")?;
    t.set_implicit(true);
    Ok(t)
}

fn set_str(t: &mut Table, k: &str, v: Option<&str>) {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => t[k] = value(s),
        None => {
            t.remove(k);
        }
    }
}

fn set_arr(t: &mut Table, k: &str, items: &[String]) {
    let kept: Vec<&str> = items.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if kept.is_empty() {
        t.remove(k);
        return;
    }
    let mut a = Array::new();
    for s in kept {
        a.push(s);
    }
    t[k] = value(a);
}

/// `original` is None when adding, Some(old_name) when editing — passing a different
/// name than the original renames the jack.
pub fn save_jack(original: Option<String>, j: JackInput) -> Result<(), String> {
    save_jack_at(&patchbay::config_path(), original, j)
}

pub fn save_jack_at(path: &Path, original: Option<String>, j: JackInput) -> Result<(), String> {
    let name = j.name.trim().to_string();
    if name.is_empty() {
        return Err("a jack needs a name".into());
    }
    if j.host.trim().is_empty() {
        return Err(format!("\"{name}\" needs a host"));
    }
    if name.contains('.') {
        return Err("a jack name can't contain a dot".into());
    }
    if let Some(u) = j.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        if !crate::is_web_url(u) {
            return Err("a url has to start with http:// or https://".into());
        }
    }

    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;

    let renaming = original.as_deref().is_some_and(|o| o != name);
    if (original.is_none() || renaming) && jacks.contains_key(&name) {
        return Err(format!("there's already a jack named \"{name}\""));
    }
    // Carried over, not dropped and rebuilt: a rename keeps the jack's comments,
    // its place in the file, and any key the sheet can't edit — same as an edit does.
    let previous = renaming
        .then(|| jacks.remove(original.as_deref().unwrap_or_default()))
        .flatten();

    let entry = jacks
        .entry(&name)
        .or_insert_with(|| previous.unwrap_or_else(|| Item::Table(Table::new())));
    let t = entry
        .as_table_mut()
        .ok_or_else(|| format!("[jack.{name}] isn't a table"))?;

    set_str(t, "host", Some(&j.host));
    set_str(t, "user", j.user.as_deref());
    set_str(t, "key", j.key.as_deref());
    set_str(t, "jump", j.jump.as_deref());
    set_str(t, "os", j.os.as_deref());
    set_str(t, "url", j.url.as_deref());
    set_str(t, "primary", j.primary.as_deref());
    set_str(t, "desc", j.desc.as_deref());
    set_arr(t, "folders", &j.folders);
    t.remove("tags");   // migrates a jack written before folders had their own key
    set_arr(t, "forward", &j.forward);
    // Only written when false; the default keeps configs uncluttered.
    match j.ssh {
        Some(false) => t["ssh"] = value(false),
        _ => {
            t.remove("ssh");
        }
    }
    match j.rdp {
        Some(p) => t["rdp"] = value(p as i64),
        None => {
            t.remove("rdp");
        }
    }
    match j.port {
        Some(p) => t["port"] = value(p as i64),
        None => {
            t.remove("port");
        }
    }

    write_doc(path, &doc)
}

/// Replace the whole config with a document from somewhere else — the team's copy.
/// Parsed before it lands, so a server handing us something unparseable can't leave
/// a broken file behind, and written the same temp-and-rename way as every other edit.
/// No plain `replace()` twin: team.rs is the only caller and it already has the path.
pub fn replace_at(path: &Path, src: &str) -> Result<(), String> {
    let doc = src
        .parse::<DocumentMut>()
        .map_err(|e| format!("the team's config doesn't parse: {e}"))?;
    write_doc(path, &doc)
}

pub fn delete_jack(name: &str) -> Result<(), String> {
    delete_jack_at(&patchbay::config_path(), name)
}

pub fn delete_jack_at(path: &Path, name: &str) -> Result<(), String> {
    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;
    if !jacks.contains_key(name) {
        return Err(format!("no jack named \"{name}\""));
    }
    let orphan = orphan_comments(jacks, name);
    rehome_comments(&mut doc, orphan);
    write_doc(path, &doc)
}

/// App preferences, in `[settings]`. Defaults are what you get with no section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Settings {
    /// Bring a folder's VPN up before connecting to a device in it.
    #[serde(default = "yes")]
    pub vpn_auto_connect: bool,
    /// Take it down when that folder's last session closes. Off by default — it
    /// will cut a tunnel you were still using outside patchbay.
    #[serde(default)]
    pub vpn_auto_disconnect: bool,
    /// TCP-probe every device's entry point on a timer for the status dots.
    #[serde(default = "yes")]
    pub probe: bool,
    /// Connect opens the system terminal instead of a tab in the window.
    #[serde(default)]
    pub connect_in_terminal: bool,
    /// Tint a device's icon by its `os`, using the brand's colour unless [colors]
    /// overrides it.
    #[serde(default = "yes")]
    pub os_colors: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vpn_auto_connect: true,
            vpn_auto_disconnect: false,
            probe: true,
            connect_in_terminal: false,
            os_colors: true,
        }
    }
}

/// `[defaults]` merges into every jack, so it accepts any jack key. The sheet only
/// offers the four worth inheriting — anything else someone wrote there by hand is
/// left exactly where it is.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Defaults {
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key: Option<String>,
    pub jump: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDefaults {
    #[serde(default)]
    defaults: Defaults,
}

pub fn load_defaults() -> Defaults {
    load_defaults_at(&patchbay::config_path())
}

/// Never fails, for the same reason `load_settings_at` doesn't: the sheet has to
/// open even when the file it is about to fix is broken.
pub fn load_defaults_at(file: &Path) -> Defaults {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| toml::from_str::<RawDefaults>(&s).ok())
        .map(|r| r.defaults)
        .unwrap_or_default()
}

pub fn save_defaults(d: &Defaults) -> Result<(), String> {
    save_defaults_at(&patchbay::config_path(), d)
}

pub fn save_defaults_at(file: &Path, d: &Defaults) -> Result<(), String> {
    let mut doc = read_doc(file)?;
    let empty = {
        let t = doc
            .entry("defaults")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or("`defaults` in the config isn't a table")?;
        set_str(t, "user", d.user.as_deref());
        set_str(t, "key", d.key.as_deref());
        set_str(t, "jump", d.jump.as_deref());
        match d.port {
            Some(p) => t["port"] = value(p as i64),
            None => {
                t.remove("port");
            }
        }
        t.is_empty()
    };
    // Nothing inherited means no section — an empty `[defaults]` left behind is
    // noise in a file people read. A hand-written key keeps the table alive.
    if empty {
        let orphan = orphan_comments(doc.as_table_mut(), "defaults");
        rehome_comments(&mut doc, orphan);
    }
    write_doc(file, &doc)
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(default)]
    settings: Settings,
}

pub fn load_settings() -> Settings {
    load_settings_at(&patchbay::config_path())
}

/// Never fails: a broken or missing config just means defaults, so the settings
/// sheet still opens and can fix whatever is wrong.
pub fn load_settings_at(file: &Path) -> Settings {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| toml::from_str::<RawSettings>(&s).ok())
        .map(|r| r.settings)
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    save_settings_at(&patchbay::config_path(), s)
}

pub fn save_settings_at(file: &Path, s: &Settings) -> Result<(), String> {
    let mut doc = read_doc(file)?;
    let t = doc
        .entry("settings")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("`settings` in the config isn't a table")?;
    t["vpn_auto_connect"] = value(s.vpn_auto_connect);
    t["vpn_auto_disconnect"] = value(s.vpn_auto_disconnect);
    t["probe"] = value(s.probe);
    t["connect_in_terminal"] = value(s.connect_in_terminal);
    t["os_colors"] = value(s.os_colors);
    write_doc(file, &doc)
}

#[derive(Debug, Default, Deserialize)]
struct RawColors {
    #[serde(default)]
    colors: std::collections::BTreeMap<String, String>,
}

/// `[colors]` maps an `os` value to a hex. Absent entries fall back to the brand's
/// own colour, so this only holds what you have deliberately changed.
pub fn load_colors() -> std::collections::BTreeMap<String, String> {
    std::fs::read_to_string(patchbay::config_path())
        .ok()
        .and_then(|s| toml::from_str::<RawColors>(&s).ok())
        .map(|r| r.colors)
        .unwrap_or_default()
}

/// `None` clears the override and restores the brand default.
pub fn save_color(os: &str, hex: Option<&str>) -> Result<(), String> {
    save_color_at(&patchbay::config_path(), os, hex)
}

pub fn save_color_at(file: &Path, os: &str, hex: Option<&str>) -> Result<(), String> {
    let os = os.trim().to_lowercase();
    if os.is_empty() {
        return Err("which os?".into());
    }
    if let Some(h) = hex {
        // This ends up in a style attribute, so nothing but a plain hex gets in.
        let ok = h.len() == 7
            && h.starts_with('#')
            && h[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            return Err(format!("\"{h}\" isn't a #rrggbb colour"));
        }
    }
    let mut doc = read_doc(file)?;
    let t = doc
        .entry("colors")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("`colors` in the config isn't a table")?;
    match hex {
        Some(h) => t[&os] = value(h),
        None => {
            t.remove(&os);
        }
    }
    write_doc(file, &doc)
}

/// `[vpn]` holds one table per folder path, same implicit-parent trick as `[jack]`.
fn vpn_table(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    let item = doc.entry("vpn").or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(true);
        Item::Table(t)
    });
    let t = item.as_table_mut().ok_or("`vpn` in the config isn't a table")?;
    t.set_implicit(true);
    Ok(t)
}

pub fn save_vpn(path: &str, v: &crate::vpn::Vpn) -> Result<(), String> {
    save_vpn_at(&patchbay::config_path(), path, v)
}

pub fn save_vpn_at(file: &Path, path: &str, v: &crate::vpn::Vpn) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("a vpn needs a folder".into());
    }
    let provider = v.provider.as_deref().unwrap_or("custom");
    if provider == "custom" {
        if v.up.as_deref().map(str::trim).unwrap_or("").is_empty() {
            return Err("a custom vpn needs an `up` command".into());
        }
    } else if v.profile.as_deref().map(str::trim).unwrap_or("").is_empty()
        && provider != "tailscale"
    {
        return Err(format!("pick a {provider} profile"));
    }

    let mut doc = read_doc(file)?;
    let vpns = vpn_table(&mut doc)?;
    let entry = vpns.entry(path).or_insert_with(|| Item::Table(Table::new()));
    let t = entry
        .as_table_mut()
        .ok_or_else(|| format!("[vpn.{path}] isn't a table"))?;
    set_str(t, "provider", Some(provider));
    set_str(t, "profile", v.profile.as_deref());
    // A preset derives these, so don't leave stale ones behind.
    let custom = provider == "custom";
    set_str(t, "up", if custom { v.up.as_deref() } else { None });
    set_str(t, "down", if custom { v.down.as_deref() } else { None });
    set_str(t, "check", if custom { v.check.as_deref() } else { None });
    write_doc(file, &doc)
}

pub fn delete_vpn(path: &str) -> Result<(), String> {
    delete_vpn_at(&patchbay::config_path(), path)
}

pub fn delete_vpn_at(file: &Path, path: &str) -> Result<(), String> {
    let mut doc = read_doc(file)?;
    let vpns = vpn_table(&mut doc)?;
    if !vpns.contains_key(path) {
        return Err(format!("no vpn on \"{path}\""));
    }
    let orphan = orphan_comments(vpns, path);
    rehome_comments(&mut doc, orphan);
    write_doc(file, &doc)
}

/// Rewrite every folder that is `path` or sits under `path/`. `to` of None deletes
/// them. This is what folder rename/delete means when a folder is only ever a string
/// on the jacks that are in it.
fn map_folders(file: &Path, path: &str, to: Option<&str>) -> Result<usize, String> {
    let mut doc = read_doc(file)?;
    let jacks = jack_table(&mut doc)?;
    let mut touched = 0;

    for (_, item) in jacks.iter_mut() {
        let Some(t) = item.as_table_mut() else { continue };
        // ponytail: `tags` is the old key for the same list, so a rename still works
        // on a file written before the change. Drop when no old files are left.
        let key = if t.contains_key("folders") { "folders" } else { "tags" };
        let Some(arr) = t.get(key).and_then(|i| i.as_array()) else { continue };

        let mut next = Array::new();
        let mut changed = false;
        for folder in arr.iter().filter_map(|v| v.as_str()) {
            let under = folder == path || folder.starts_with(&format!("{path}/"));
            if !under {
                next.push(folder);
                continue;
            }
            changed = true;
            if let Some(to) = to {
                next.push(format!("{to}{}", &folder[path.len()..]).as_str());
            }
        }
        if changed {
            touched += 1;
            if next.is_empty() {
                t.remove(key);
            } else {
                t[key] = value(next);
            }
        }
    }

    if touched > 0 {
        write_doc(file, &doc)?;
    }
    Ok(touched)
}

pub fn rename_group(from: &str, to: &str) -> Result<usize, String> {
    rename_group_at(&patchbay::config_path(), from, to)
}

pub fn rename_group_at(file: &Path, from: &str, to: &str) -> Result<usize, String> {
    let to = to.trim().trim_matches('/');
    if to.is_empty() {
        return Err("a folder needs a name".into());
    }
    let touched = map_folders(file, from, Some(to))?;
    // A [vpn."old/path"] would otherwise be orphaned by the rename.
    let mut doc = read_doc(file)?;
    let vpns = vpn_table(&mut doc)?;
    let moved: Vec<String> = vpns
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| k == from || k.starts_with(&format!("{from}/")))
        .collect();
    if !moved.is_empty() {
        for key in moved {
            if let Some(item) = vpns.remove(&key) {
                vpns.insert(&format!("{to}{}", &key[from.len()..]), item);
            }
        }
        write_doc(file, &doc)?;
    }
    Ok(touched)
}

pub fn delete_group(path: &str) -> Result<usize, String> {
    delete_group_at(&patchbay::config_path(), path)
}

pub fn delete_group_at(file: &Path, path: &str) -> Result<usize, String> {
    map_folders(file, path, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# my hosts — keep this comment
[defaults]
user = "root"          # trailing comment

# the way in
[jack.bastion]
host = "bastion.example"
port = 2222
folders = ["prod/eu", "entrypoint"]

[jack.web]
host = "10.0.0.4"
jump = "bastion"
folders = ["prod/eu/web"]
"#;

    fn scratch(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("patchbay-{}-{name}.toml", std::process::id()));
        std::fs::write(&p, SAMPLE).unwrap();
        p
    }
    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }
    fn input(name: &str, host: &str) -> JackInput {
        JackInput {
            name: name.into(), host: host.into(),
            user: None, port: None, key: None, jump: None, os: None, url: None, rdp: None, ssh: None, primary: None, desc: None,
            folders: vec![], forward: vec![],
        }
    }

    #[test]
    fn defaults_round_trip_and_vanish_once_nothing_is_set() {
        let p = scratch("defaults");
        let d = Defaults {
            user: Some("ops".into()),
            port: Some(2222),
            key: None,
            jump: None,
        };
        save_defaults_at(&p, &d).unwrap();

        let back = load_defaults_at(&p);
        assert_eq!(back.user.as_deref(), Some("ops"));
        assert_eq!(back.port, Some(2222));
        assert!(read(&p).contains("keep this comment"));

        // Clearing every field takes the section with it rather than leaving an
        // empty `[defaults]` in a file people read.
        save_defaults_at(&p, &Defaults::default()).unwrap();
        assert!(!read(&p).contains("[defaults]"), "got {}", read(&p));
    }

    #[test]
    fn comments_outlive_the_table_they_sat_above() {
        let p = scratch("comments");
        save_defaults_at(&p, &Defaults::default()).unwrap();
        let out = read(&p);
        assert!(!out.contains("[defaults]"), "got {out}");
        assert!(out.contains("# my hosts"), "the file header went with it:\n{out}");
        assert!(
            out.find("# my hosts") < out.find("[jack.bastion]"),
            "the header should still be on top:\n{out}"
        );

        delete_jack_at(&p, "bastion").unwrap();
        let out = read(&p);
        assert!(out.contains("# my hosts"), "got {out}");
        assert!(out.contains("# the way in"), "got {out}");
        assert!(out.find("# my hosts") < out.find("[jack.web]"), "got {out}");

        // Nothing renders after the last jack, so its comments end up at the end
        // rather than nowhere.
        delete_jack_at(&p, "web").unwrap();
        assert!(read(&p).contains("# my hosts"), "got {}", read(&p));
    }

    #[test]
    fn renaming_a_jack_takes_its_comment_along() {
        let p = scratch("rename-comment");
        save_jack_at(&p, Some("bastion".into()), input("gateway", "bastion.example")).unwrap();
        let out = read(&p);
        assert!(out.contains("# the way in"), "got {out}");
        assert!(out.find("# the way in") < out.find("[jack.gateway]"), "got {out}");
    }

    #[test]
    fn a_hand_written_default_the_sheet_cannot_edit_survives() {
        let p = scratch("defaults-extra");
        std::fs::write(&p, "[defaults]\nuser = \"root\"\nos = \"debian\"\n").unwrap();

        save_defaults_at(&p, &Defaults::default()).unwrap();

        let out = read(&p);
        assert!(out.contains("os = \"debian\""), "got {out}");
        assert!(!out.contains("user"), "got {out}");
    }

    #[test]
    fn adding_a_jack_keeps_every_comment_and_existing_key() {
        let p = scratch("add");
        save_jack_at(&p, None, input("new-box", "10.0.0.9")).unwrap();
        let out = read(&p);
        assert!(out.contains("# my hosts — keep this comment"));
        assert!(out.contains("# trailing comment"));
        assert!(out.contains("# the way in"));
        assert!(out.contains(r#"port = 2222"#));
        assert!(out.contains(r#"[jack.new-box]"#));
        assert!(out.contains(r#"host = "10.0.0.9""#));
    }

    #[test]
    fn editing_clears_keys_that_were_emptied() {
        let p = scratch("edit");
        let mut j = input("bastion", "bastion.example");
        j.folders = vec!["prod/eu".into()];
        save_jack_at(&p, Some("bastion".into()), j).unwrap();
        let out = read(&p);
        assert!(!out.contains("port = 2222"), "cleared port should be gone:\n{out}");
        assert!(out.contains("# the way in"), "comment survived the edit");
        assert!(!out.contains("entrypoint"), "dropped tag should be gone");
    }

    #[test]
    fn rename_moves_the_jack_and_rejects_collisions() {
        let p = scratch("rename");
        save_jack_at(&p, Some("web".into()), input("web2", "10.0.0.4")).unwrap();
        let out = read(&p);
        assert!(out.contains("[jack.web2]"));
        assert!(!out.contains("[jack.web]\n"));

        let err = save_jack_at(&p, None, input("bastion", "x")).unwrap_err();
        assert!(err.contains("already a jack named"), "got {err}");
    }

    #[test]
    fn a_jack_needs_a_name_and_a_host() {
        let p = scratch("valid");
        assert!(save_jack_at(&p, None, input("", "h")).unwrap_err().contains("needs a name"));
        assert!(save_jack_at(&p, None, input("x", "  ")).unwrap_err().contains("needs a host"));
        // a dot would nest it under another table
        assert!(save_jack_at(&p, None, input("a.b", "h")).unwrap_err().contains("dot"));
    }

    #[test]
    fn renaming_a_group_rewrites_the_whole_subtree() {
        let p = scratch("group-rename");
        assert_eq!(rename_group_at(&p, "prod/eu", "prod/emea").unwrap(), 2);
        let out = read(&p);
        assert!(out.contains(r#""prod/emea""#), "{out}");
        assert!(out.contains(r#""prod/emea/web""#), "children move too:\n{out}");
        assert!(out.contains(r#""entrypoint""#), "unrelated folders untouched");
    }

    #[test]
    fn deleting_a_group_drops_the_folders_but_keeps_the_jacks() {
        let p = scratch("group-delete");
        assert_eq!(delete_group_at(&p, "prod/eu").unwrap(), 2);
        let out = read(&p);
        assert!(!out.contains("prod/eu"));
        assert!(out.contains("[jack.bastion]"), "the device stays");
        assert!(out.contains("[jack.web]"), "the device stays");
        assert!(out.contains(r#"folders = ["entrypoint"]"#), "its other folder stays");
        // web's only tag was under the folder, so the key goes entirely
        assert!(!out.contains(r#"folders = []"#));
    }

    #[test]
    fn vpn_round_trips_and_needs_an_up_command() {
        let p = scratch("vpn");
        let custom = |up: Option<&str>, down: Option<&str>, check: Option<&str>| crate::vpn::Vpn {
            provider: Some("custom".into()),
            up: up.map(str::to_string),
            down: down.map(str::to_string),
            check: check.map(str::to_string),
            ..Default::default()
        };
        save_vpn_at(&p, "acme", &custom(Some("wg-quick up acme"), Some("wg-quick down acme"), None)).unwrap();
        let out = read(&p);
        assert!(out.contains("[vpn.acme]"), "{out}");
        assert!(out.contains(r#"up = "wg-quick up acme""#));
        assert!(!out.contains("check"), "an empty check shouldn't be written");
        assert!(out.contains("# my hosts — keep this comment"), "comments survive");

        // clearing a field removes the key rather than writing ""
        save_vpn_at(&p, "acme", &custom(Some("x"), None, Some("wg show acme"))).unwrap();
        let out = read(&p);
        assert!(!out.contains("down ="), "{out}");
        assert!(out.contains(r#"check = "wg show acme""#));

        assert!(save_vpn_at(&p, "acme", &custom(None, None, None)).unwrap_err().contains("`up`"));
        assert!(save_vpn_at(&p, "  ", &custom(Some("x"), None, None)).unwrap_err().contains("folder"));

        delete_vpn_at(&p, "acme").unwrap();
        assert!(!read(&p).contains("[vpn.acme]"));
        assert!(delete_vpn_at(&p, "acme").unwrap_err().contains("no vpn"));
    }

    #[test]
    fn renaming_a_folder_carries_its_vpn_across() {
        let p = scratch("vpn-rename");
        save_vpn_at(&p, "prod/eu", &crate::vpn::Vpn {
            provider: Some("custom".into()), up: Some("up".into()), ..Default::default()
        }).unwrap();
        rename_group_at(&p, "prod/eu", "prod/emea").unwrap();
        let out = read(&p);
        assert!(out.contains(r#"[vpn."prod/emea"]"#), "{out}");
        assert!(!out.contains(r#"[vpn."prod/eu"]"#));
    }

    #[test]
    fn colours_are_validated_and_clearable() {
        let p = scratch("colors");
        save_color_at(&p, "Synology", Some("#0C4A9F")).unwrap();
        let out = read(&p);
        assert!(out.contains("[colors]"), "{out}");
        assert!(out.contains(r##"synology = "#0C4A9F""##), "key is lowercased: {out}");

        for bad in ["blue", "#0C4A9", "#GGGGGG", "red; background:url(x)"] {
            assert!(save_color_at(&p, "x", Some(bad)).is_err(), "{bad:?} should be rejected");
        }

        save_color_at(&p, "synology", None).unwrap();
        assert!(!read(&p).contains("synology ="));
    }

    #[test]
    fn deleting_a_jack_reports_an_unknown_name() {
        let p = scratch("delete");
        delete_jack_at(&p, "web").unwrap();
        assert!(!read(&p).contains("[jack.web]"));
        assert!(delete_jack_at(&p, "web").unwrap_err().contains("no jack named"));
    }

    #[test]
    fn a_missing_config_is_created_rather_than_erroring() {
        let p = std::env::temp_dir().join(format!("patchbay-{}-fresh/patchbay.toml", std::process::id()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        save_jack_at(&p, None, input("first", "10.0.0.1")).unwrap();
        assert!(read(&p).contains("[jack.first]"));
    }
}
