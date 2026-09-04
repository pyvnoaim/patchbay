//! Writing the config back out. The file is something people hand-edit, so every
//! change goes through toml_edit - comments, spacing and key order survive - and
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
    pub vnc: Option<u16>,
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

/// The name of the file patchbay writes into `~/.ssh`, and the `Include` line that
/// makes ssh read it. Relative, because ssh resolves a relative Include against `~/.ssh`
/// and an absolute one would bake this machine's home directory into a line people
/// carry between machines.
const SSH_FILE: &str = "patchbay.conf";
const SSH_INCLUDE: &str = "Include patchbay.conf";

/// Write the generated host list and make sure `~/.ssh/config` reads it.
///
/// Its own file, never theirs: people hand-tune that config for years and it is not
/// ours to rewrite. The one thing we touch in it is a single `Include` at the top, and
/// the top is where a first-wins file wants it - `to_ssh_config` has already left out
/// every name their config spells out, so nothing of theirs is shadowed from up there.
pub fn write_ssh_include(dir: &Path, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let ours = dir.join(SSH_FILE);
    // Nothing to do is the common case - this runs whenever the list is read.
    if std::fs::read_to_string(&ours).is_ok_and(|had| had == body) {
        return Ok(());
    }
    let tmp = dir.join(format!("{SSH_FILE}.tmp"));
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &ours).map_err(|e| format!("{}: {e}", ours.display()))?;

    let cfg = dir.join("config");
    let had = std::fs::read_to_string(&cfg).unwrap_or_default();
    if includes_ours(&had) {
        return Ok(());
    }
    write_text(&cfg, &format!("{SSH_INCLUDE}\n\n{had}"))
}

/// Put it back the way it was: the generated file goes, and so does the one line we
/// added. Everything else in their config is left exactly where they wrote it.
pub fn remove_ssh_include(dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(dir.join(SSH_FILE));
    let cfg = dir.join("config");
    let Ok(had) = std::fs::read_to_string(&cfg) else {
        return Ok(());
    };
    if !includes_ours(&had) {
        return Ok(());
    }
    let kept: Vec<&str> = had.lines().filter(|l| !is_our_include(l)).collect();
    write_text(&cfg, &format!("{}\n", kept.join("\n").trim_start()))
}

/// What a previous install left behind in `~/.ssh`, so the window can offer to sweep
/// it up when this setting has been off since (re)install. The two answers are
/// independent: someone might have deleted `patchbay.conf` themselves and left the
/// `Include` line, or the file might be here without a line reading it.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct Leftovers {
    pub conf_file: bool,
    pub include_line: bool,
    /// Absolute paths, so the pill can show them and the user knows what will go.
    pub conf_path: String,
    pub config_path: String,
}

pub fn ssh_leftovers(dir: &Path) -> Leftovers {
    let conf = dir.join(SSH_FILE);
    let cfg = dir.join("config");
    Leftovers {
        conf_file: conf.is_file(),
        include_line: std::fs::read_to_string(&cfg).is_ok_and(|s| includes_ours(&s)),
        conf_path: conf.display().to_string(),
        config_path: cfg.display().to_string(),
    }
}

/// Spelled either way people write it - ours goes in relative, but someone who has
/// moved it to an absolute path still has it, and adding a second line would be worse
/// than leaving theirs alone.
///
/// The *whole* last segment, never just the tail: `Include ~/.ssh/work-patchbay.conf`
/// is someone else's file, and reading it as ours would take their line out of their
/// config the first time this is switched off.
fn is_our_include(line: &str) -> bool {
    let l = line.trim();
    let Some(path) = l.strip_prefix("Include ").or_else(|| l.strip_prefix("include ")) else {
        return false;
    };
    path.trim().rsplit('/').next() == Some(SSH_FILE)
}

fn includes_ours(src: &str) -> bool {
    src.lines().any(is_our_include)
}

/// Temp file and rename, like every other write here - someone's ssh config is not a
/// thing to leave half-written.
fn write_text(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension("patchbay-tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Same directory, so the rename is atomic - the config is never half-written.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// toml_edit hangs the lines above a table on that table, so removing one deletes
/// the comments sitting over it - including a file header that was never about it.
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
/// place is the next position along - wherever in the tree it happens to live.
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

/// `[jack]` is implicit - we only ever write the `[jack.name]` children.
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

/// `original` is None when adding, Some(old_name) when editing - passing a different
/// name than the original renames the jack.
pub fn save_jack_at(path: &Path, original: Option<String>, j: JackInput) -> Result<(), String> {
    let name = j.name.trim().to_string();
    if name.is_empty() {
        return Err("a jack needs a name".into());
    }
    if j.host.trim().is_empty() {
        return Err(format!("\"{name}\" needs a host"));
    }
    if let Some(u) = j.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        if !crate::is_web_url(u) {
            return Err("a url has to start with http:// or https://".into());
        }
    }
    // Checked on the way in as well as on the way out, the way a url is: a forward
    // becomes argv, and finding out it wasn't one at connect time means a device that
    // was saved and simply never works.
    for f in &j.forward {
        crate::patchbay::forward_arg(f)?;
    }

    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;

    let renaming = original.as_deref().is_some_and(|o| o != name);
    if (original.is_none() || renaming) && jacks.contains_key(&name) {
        return Err(format!("there's already a jack named \"{name}\""));
    }
    // Carried over, not dropped and rebuilt: a rename keeps the jack's comments,
    // its place in the file, and any key the sheet can't edit - same as an edit does.
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
    match j.vnc {
        Some(p) => t["vnc"] = value(p as i64),
        None => {
            t.remove("vnc");
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

/// A folder's note: the thing people keep a README in the tree for. Blank removes it,
/// and the whole `[folder]` table goes with the last one - a file full of empty tables
/// is worse than no feature.
pub fn set_note_at(file: &Path, folder: &str, note: &str) -> Result<(), String> {
    let path = folder.trim().trim_matches('/');
    if path.is_empty() {
        return Err("a note belongs to a folder".into());
    }
    let mut doc = read_doc(file)?;
    match note.trim().is_empty() {
        true => {
            if let Some(t) = doc.get_mut("folder").and_then(Item::as_table_mut) {
                t.remove(path);
                if t.is_empty() {
                    doc.remove("folder");
                }
            }
        }
        false => {
            let table = doc
                .entry("folder")
                .or_insert_with(|| {
                    let mut t = Table::new();
                    t.set_implicit(true);
                    Item::Table(t)
                })
                .as_table_mut()
                .ok_or("`folder` in that file is not a table")?;
            table[path]["note"] = value(note.trim());
        }
    }
    write_doc(file, &doc)
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
    /// TCP-probe every device's entry point on a timer for the status dots.
    #[serde(default = "yes")]
    pub probe: bool,
    /// Connect opens the system terminal instead of a tab in the window.
    #[serde(default)]
    pub connect_in_terminal: bool,
    /// Write the device list into `~/.ssh/patchbay.conf` and have `~/.ssh/config`
    /// include it, so `ssh web-01` in any terminal reaches what Connect reaches. Off by
    /// default: it is the one setting that writes outside patchbay's own directory.
    #[serde(default)]
    pub write_ssh_config: bool,
    /// Tint a device's icon by its `os`, using the brand's colour unless [colors]
    /// overrides it.
    #[serde(default = "yes")]
    pub os_colors: bool,
    /// Ask the update endpoint once per launch. On unless it is turned off here -
    /// it is the only request the app makes on its own.
    #[serde(default = "yes")]
    pub check_updates: bool,
    /// Append every session's terminal output to a file under `logs/` beside the
    /// config, for whoever wants a record of what ran on a box. Off by default,
    /// same reasoning as `write_ssh_config`: it writes outside patchbay's own file.
    #[serde(default)]
    pub log_sessions: bool,
    /// "system" follows the machine; "light" and "dark" pin it. Anything else reads
    /// as "system", so a typo here is a working app rather than an unstyled one.
    #[serde(default = "system")]
    pub theme: String,
    /// Terminal font size, in px. Clamped on the way in - it reaches xterm, which
    /// will happily lay out a session at 400px.
    #[serde(default = "font_size")]
    pub font_size: f64,
    /// The sidebar's width in px, as you last dragged it. Clamped here too: it lands
    /// in a grid template, and a column wider than the window leaves no list.
    #[serde(default = "sidebar")]
    pub sidebar: f64,
}

fn yes() -> bool {
    true
}

fn system() -> String {
    "system".into()
}

fn font_size() -> f64 {
    12.5
}

fn sidebar() -> f64 {
    208.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            probe: true,
            connect_in_terminal: false,
            write_ssh_config: false,
            os_colors: true,
            check_updates: true,
            log_sessions: false,
            theme: system(),
            font_size: font_size(),
            sidebar: sidebar(),
        }
    }
}

/// `[defaults]` merges into every jack, so it accepts any jack key. The sheet only
/// offers the four worth inheriting - anything else someone wrote there by hand is
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
    // Nothing inherited means no section - an empty `[defaults]` left behind is
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
    t["probe"] = value(s.probe);
    t["connect_in_terminal"] = value(s.connect_in_terminal);
    t["os_colors"] = value(s.os_colors);
    t["check_updates"] = value(s.check_updates);
    t["log_sessions"] = value(s.log_sessions);
    // Both are read straight back out by the window - one onto the root element, one
    // into xterm - so they are narrowed here rather than wherever they land.
    t["theme"] = value(match s.theme.as_str() {
        "light" => "light",
        "dark" => "dark",
        _ => "system",
    });
    t["font_size"] = value(s.font_size.clamp(8.0, 32.0));
    t["sidebar"] = value(s.sidebar.clamp(150.0, 480.0));
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

/// on the jacks that are in it.
/// A note is hung on a path, so a folder that moves has to take it along and one that
/// goes has to drop it - otherwise a rename silently orphans what somebody wrote.
fn move_note(doc: &mut DocumentMut, from: &str, to: Option<&str>) {
    let Some(table) = doc.get_mut("folder").and_then(Item::as_table_mut) else { return };
    let moved: Vec<(String, Item)> = table
        .iter()
        .filter(|(k, _)| *k == from || k.starts_with(&format!("{from}/")))
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    for (key, item) in moved {
        table.remove(&key);
        if let Some(to) = to {
            table.insert(&format!("{to}{}", &key[from.len()..]), item);
        }
    }
    if table.is_empty() {
        doc.remove("folder");
    }
}

fn map_folders(file: &Path, path: &str, to: Option<&str>) -> Result<usize, String> {
    let mut doc = read_doc(file)?;
    move_note(&mut doc, path, to);
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

pub fn rename_group_at(file: &Path, from: &str, to: &str) -> Result<usize, String> {
    let to = to.trim().trim_matches('/');
    if to.is_empty() {
        return Err("a folder needs a name".into());
    }
    map_folders(file, from, Some(to))
}

pub fn delete_group_at(file: &Path, path: &str) -> Result<usize, String> {
    map_folders(file, path, None)
}

/// Spaces were extra config files beside the main one, from when a shared list had to
/// be a whole file of its own. One list and folders do that job now, so anything still
/// in `spaces/` is folded in on the way past: each device keeps its devices' folders
/// with the space's name in front, so `acme` + `prod` becomes `acme/prod` and nothing
/// that was filed separately ends up mixed in.
///
/// Runs once - the files it reads are renamed `.toml.merged` rather than deleted,
/// because it is somebody's device list and this is the only copy of the split version.
pub fn fold_spaces_at(cfg: &Path) -> Result<usize, String> {
    let dir = cfg.with_file_name("spaces");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(0);
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(0);
    }

    let mut doc = read_doc(cfg)?;
    let (mut moved, mut merged) = (0, Vec::new());
    for file in files {
        let Some(space) = file.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let Ok(src) = std::fs::read_to_string(&file) else { continue };
        let Ok(from) = src.parse::<DocumentMut>() else {
            // A file that doesn't parse is left exactly where it is, named in the log
            // rather than quietly dropped on the floor.
            eprintln!("patchbay: {} doesn't parse, so it was left alone", file.display());
            continue;
        };
        let Some(jacks) = from.get("jack").and_then(Item::as_table) else { continue };
        // A space had its own `[defaults]`, which applied to its devices and nobody
        // else's. There is one `[defaults]` after this, so the inherited keys are
        // written onto each device on the way over - otherwise a device that leaned on
        // `user = "root"` in its own file arrives without a user and simply stops
        // working, which is the worst way for a migration to fail.
        let defaults = from.get("defaults").and_then(Item::as_table);

        for (name, item) in jacks.iter() {
            let Some(t) = item.as_table() else { continue };
            let mut t = t.clone();
            if let Some(d) = defaults {
                for (k, v) in d.iter() {
                    if !t.contains_key(k) {
                        t.insert(k, v.clone());
                    }
                }
            }
            // The space becomes the outermost folder, so the split survives as a
            // branch of the tree instead of as a second file.
            let key = if t.contains_key("folders") { "folders" } else { "tags" };
            let mut folders = Array::new();
            match t.get(key).and_then(Item::as_array) {
                Some(had) if !had.is_empty() => {
                    for v in had.iter().filter_map(|v| v.as_str()) {
                        folders.push(format!("{space}/{v}"));
                    }
                }
                _ => folders.push(space.clone()),
            }
            t.remove("tags");
            t["folders"] = Item::Value(folders.into());

            let table = doc
                .entry("jack")
                .or_insert_with(|| {
                    let mut t = Table::new();
                    t.set_implicit(true);
                    Item::Table(t)
                })
                .as_table_mut()
                .ok_or("`jack` in that config is not a table")?;
            // First wins, the way loading two spaces did: a name already here is the
            // one you have been using.
            let mut at = name.to_string();
            let mut n = 2;
            while table.contains_key(&at) {
                at = format!("{name} {n}");
                n += 1;
            }
            table.insert(&at, Item::Table(t));
            moved += 1;
        }
        merged.push(file);
    }
    if moved == 0 {
        return Ok(0);
    }
    // The config lands before anything is renamed. The other order loses the devices
    // outright if this write fails: the files it read would already be `.toml.merged`,
    // and the list they were folded into was never written.
    write_doc(cfg, &doc)?;
    for file in merged {
        let _ = std::fs::rename(&file, file.with_extension("toml.merged"));
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# my hosts - keep this comment
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
            user: None, port: None, key: None, jump: None, os: None, url: None, rdp: None, vnc: None, ssh: None, primary: None, desc: None,
            folders: vec![], forward: vec![],
        }
    }

    #[test]
    fn a_theme_and_a_font_size_are_narrowed_on_the_way_in() {
        let p = scratch("settings");
        save_settings_at(&p, &Settings { theme: "neon".into(), font_size: 900.0, ..Settings::default() }).unwrap();
        let back = load_settings_at(&p);
        // One reaches the root element, the other reaches xterm's layout.
        assert_eq!(back.theme, "system");
        assert_eq!(back.font_size, 32.0);

        save_settings_at(&p, &Settings { theme: "light".into(), font_size: 14.0, ..Settings::default() }).unwrap();
        let back = load_settings_at(&p);
        assert_eq!(back.theme, "light");
        assert_eq!(back.font_size, 14.0);
        assert!(read(&p).contains("keep this comment"));
    }

    #[test]
    fn the_update_check_is_on_until_it_is_turned_off() {
        let p = scratch("check_updates");
        // A config written before the setting existed still checks - absent is on,
        // and a machine that silently stopped looking would never say so.
        assert!(load_settings_at(&p).check_updates);

        save_settings_at(&p, &Settings { check_updates: false, ..Settings::default() }).unwrap();
        assert!(!load_settings_at(&p).check_updates);
    }

    #[test]
    fn session_logging_is_off_until_it_is_turned_on() {
        let p = scratch("log_sessions");
        assert!(!load_settings_at(&p).log_sessions);

        save_settings_at(&p, &Settings { log_sessions: true, ..Settings::default() }).unwrap();
        assert!(load_settings_at(&p).log_sessions);
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
        assert!(out.contains("# my hosts - keep this comment"));
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
    }

    /// The thing people keep a README in the tree for. It hangs on the path, so the
    /// two things that move a path have to carry it - a rename that orphaned somebody's
    /// notes would be a silent loss of the only writing in the file.
    /// The one-way door out of spaces. It has to be lossless in the way that matters:
    /// every device arrives, and the split it used to have survives as a branch rather
    /// than dissolving into everyone else's list.
    #[test]
    fn spaces_fold_into_the_one_list_and_keep_their_shape() {
        let cfg = std::env::temp_dir().join(format!("patchbay-{}-fold/patchbay.toml", std::process::id()));
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
        std::fs::create_dir_all(cfg.with_file_name("spaces")).unwrap();
        std::fs::write(&cfg, "# mine\n[jack.laptop]\nhost = \"192.168.1.9\"\n").unwrap();
        std::fs::write(
            cfg.with_file_name("spaces").join("acme.toml"),
            "[defaults]\nuser = \"root\"\n\n[jack.db]\nhost = \"10.0.0.5\"\n\
             [jack.web]\nhost = \"10.0.0.4\"\nuser = \"deploy\"\nfolders = [\"prod\"]\n",
        )
        .unwrap();
        // A name that is already in the main list, which first-wins used to hide.
        std::fs::write(
            cfg.with_file_name("spaces").join("lab.toml"),
            "[jack.laptop]\nhost = \"10.1.1.1\"\n",
        )
        .unwrap();

        assert_eq!(fold_spaces_at(&cfg).unwrap(), 3);
        let jacks = patchbay::parse(&read(&cfg)).unwrap();
        assert_eq!(jacks.len(), 4);
        assert!(read(&cfg).contains("# mine"), "the file was re-serialized");

        // A space's `[defaults]` applied to its devices and to nobody else's, and there
        // is only one `[defaults]` after this - so what they inherited comes with them,
        // and what they set themselves is left alone.
        assert_eq!(jacks["db"].user.as_deref(), Some("root"), "an inherited user was dropped");
        assert_eq!(jacks["web"].user.as_deref(), Some("deploy"), "an inherited user won");

        // The space is the outermost folder now, whether or not there was one before.
        assert_eq!(jacks["db"].folders.as_deref(), Some(&["acme".to_string()][..]));
        assert_eq!(jacks["web"].folders.as_deref(), Some(&["acme/prod".to_string()][..]));
        // Yours is untouched and the other one is beside it under a name of its own.
        assert_eq!(jacks["laptop"].host, "192.168.1.9");
        assert_eq!(jacks["laptop 2"].host, "10.1.1.1");

        // The files it read are kept, and it does not run twice.
        assert!(cfg.with_file_name("spaces").join("acme.toml.merged").exists());
        assert_eq!(fold_spaces_at(&cfg).unwrap(), 0);
    }

    #[test]
    fn a_folder_note_survives_a_rename_and_goes_with_a_delete() {
        let p = scratch("notes");
        set_note_at(&p, "prod/eu", "the recovery key is in the safe\nask Anna first").unwrap();
        assert_eq!(
            patchbay::notes(&read(&p))["prod/eu"],
            "the recovery key is in the safe\nask Anna first",
            "a note has to survive a round trip with its line breaks"
        );

        rename_group_at(&p, "prod/eu", "prod/emea").unwrap();
        let after = patchbay::notes(&read(&p));
        assert!(after.contains_key("prod/emea"), "the note was orphaned by a rename");
        assert!(!after.contains_key("prod/eu"));

        delete_group_at(&p, "prod/emea").unwrap();
        assert!(patchbay::notes(&read(&p)).is_empty());
        assert!(!read(&p).contains("[folder"), "an empty table was left behind");

        // And a blank note is a removal, not a folder with an empty string in it.
        set_note_at(&p, "prod", "x").unwrap();
        set_note_at(&p, "prod", "  ").unwrap();
        assert!(patchbay::notes(&read(&p)).is_empty());
        assert!(read(&p).contains("keep this comment"), "the file was re-serialized");
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

    /// The one write that lands outside patchbay's own directory. Their config is
    /// theirs: one line goes in at the top, and turning it off takes exactly that line
    /// and the generated file, leaving everything they wrote where they wrote it.
    #[test]
    fn the_ssh_include_is_one_line_of_theirs_and_comes_back_out_cleanly() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-sshinc", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        let mine = "Host mine\n  HostName 10.0.0.2\n";
        std::fs::write(&cfg, mine).unwrap();

        write_ssh_include(&dir, "Host db\n").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("patchbay.conf")).unwrap(), "Host db\n");
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(after.starts_with("Include patchbay.conf\n"), "at the top: {after:?}");
        assert!(after.contains(mine), "everything they wrote is still there");

        // Run again and it is the same file - this is called on every read of the list.
        write_ssh_include(&dir, "Host db\n").unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), after, "no second Include");

        remove_ssh_include(&dir).unwrap();
        assert!(!dir.join("patchbay.conf").exists());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), mine, "theirs, untouched");
    }

    /// The line we take back out is *our* file, not anything whose name happens to end
    /// the same way - taking someone's own Include out of their config would be the one
    /// thing this feature promised never to do.
    #[test]
    fn only_our_own_include_line_is_ours_to_remove() {
        assert!(is_our_include("Include patchbay.conf"));
        assert!(is_our_include("  include ~/.ssh/patchbay.conf  "));
        assert!(!is_our_include("Include ~/.ssh/work-patchbay.conf"));
        assert!(!is_our_include("Include conf.d/*.conf"));
        assert!(!is_our_include("Host patchbay.conf"));
    }

    /// Nothing of ours in there yet is the first-run case, and the common one for
    /// anyone who has never written an ssh config by hand.
    #[test]
    fn a_machine_with_no_ssh_config_gets_one_with_only_the_include_in_it() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-sshnew", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        write_ssh_include(&dir, "Host db\n").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("config")).unwrap(), "Include patchbay.conf\n\n");
        remove_ssh_include(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("config")).unwrap().trim(), "");
    }

    /// The reason `ssh_leftovers` exists: uninstall is drag-to-trash on macOS, so a
    /// previous install of patchbay can leave the file and the Include line behind.
    /// The window offers to clean up on first launch of the next install.
    #[test]
    fn a_previous_install_can_be_swept_up_and_a_clean_machine_says_so() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-sshleft", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Nothing left behind: both false.
        let l = ssh_leftovers(&dir);
        assert!(!l.conf_file && !l.include_line);
        // Simulate an install that turned the setting on once.
        write_ssh_include(&dir, "Host db\n").unwrap();
        let l = ssh_leftovers(&dir);
        assert!(l.conf_file && l.include_line, "the file and the include line are both here");
        assert!(l.conf_path.ends_with("patchbay.conf"));
        // Someone deleted the file by hand but left the line in ~/.ssh/config -
        // one leftover is still a leftover.
        std::fs::remove_file(dir.join(SSH_FILE)).unwrap();
        let l = ssh_leftovers(&dir);
        assert!(!l.conf_file && l.include_line);
        // The cleanup takes them both out and stays quiet on a machine that has neither.
        remove_ssh_include(&dir).unwrap();
        let l = ssh_leftovers(&dir);
        assert!(!l.conf_file && !l.include_line);
    }

    #[test]
    fn a_missing_config_is_created_rather_than_erroring() {
        let p = std::env::temp_dir().join(format!("patchbay-{}-fresh/patchbay.toml", std::process::id()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        save_jack_at(&p, None, input("first", "10.0.0.1")).unwrap();
        assert!(read(&p).contains("[jack.first]"));
    }

}
