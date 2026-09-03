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
    if name.contains('.') {
        return Err("a jack name can't contain a dot".into());
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

/// Replace the whole config with a document from somewhere else - the team's copy.
/// Parsed before it lands, so a server handing us something unparseable can't leave
/// a broken file behind, and written the same temp-and-rename way as every other edit.
/// No plain `replace()` twin: team.rs is the only caller and it already has the path.
pub fn replace_at(path: &Path, src: &str) -> Result<(), String> {
    let doc = src
        .parse::<DocumentMut>()
        .map_err(|e| format!("the team's config doesn't parse: {e}"))?;
    write_doc(path, &doc)
}

/// Move one device's table between two space files, comments and all. Not a
/// save-then-delete through `JackInput`: that would bake the source space's
/// `[defaults]` into the jack and drop any key the sheet can't edit.
pub fn move_jack_at(from: &Path, to: &Path, name: &str) -> Result<(), String> {
    if from == to {
        return Ok(());
    }
    let mut src = read_doc(from)?;
    let table = jack_table(&mut src)?
        .get(name)
        .and_then(Item::as_table)
        .ok_or_else(|| format!("no jack named \"{name}\""))?
        .clone();

    let mut dst = read_doc(to)?;
    let jacks = jack_table(&mut dst)?;
    if jacks.contains_key(name) {
        return Err(format!("there's already a jack named \"{name}\" there"));
    }
    // Rebuilt rather than moved whole: a table carries the position it had in the
    // old file, which means nothing in the new one. The keys keep their own decor,
    // and the comment above the jack is carried across by hand.
    let mut moved = Table::new();
    for (k, v) in table.iter() {
        moved.insert(k, v.clone());
    }
    if let Some(prefix) = table.decor().prefix().and_then(|p| p.as_str()) {
        moved.decor_mut().set_prefix(prefix.to_string());
    }
    jacks.insert(name, Item::Table(moved));

    // The copy lands first. If the removal then fails the device exists in both
    // files, which is visible and fixable; the other order loses it.
    write_doc(to, &dst)?;
    delete_jack_at(from, name)
}

/// A space's name becomes a file name, so it is checked as one. No dots, which is
/// what keeps `..` and a second extension out of it.
pub fn space_slug(name: &str) -> Result<String, String> {
    let s = name.trim();
    if s.is_empty() {
        return Err("a space needs a name".into());
    }
    if s.chars().count() > 40 {
        return Err("a space name has to be shorter than that".into());
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || " -_".contains(c)) {
        return Err(format!("\"{s}\" can only have letters, digits, spaces, - and _"));
    }
    Ok(s.to_string())
}

pub fn create_space_at(cfg: &Path, name: &str) -> Result<String, String> {
    let slug = space_slug(name)?;
    let path = patchbay::space_path(cfg, Some(&slug));
    if path.exists() {
        return Err(format!("there's already a space called \"{slug}\""));
    }
    write_doc(&path, &DocumentMut::new())?;
    Ok(slug)
}

/// Kept as a `.bak`, never unlinked - the file is somebody's device list, and it is
/// the same reasoning the team code's `backup` runs on.
pub fn delete_space_at(cfg: &Path, name: &str) -> Result<(), String> {
    let slug = space_slug(name)?;
    let path = patchbay::space_path(cfg, Some(&slug));
    std::fs::rename(&path, path.with_extension("toml.bak"))
        .map_err(|e| format!("{}: {e}", path.display()))
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

    #[test]
    fn a_missing_config_is_created_rather_than_erroring() {
        let p = std::env::temp_dir().join(format!("patchbay-{}-fresh/patchbay.toml", std::process::id()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        save_jack_at(&p, None, input("first", "10.0.0.1")).unwrap();
        assert!(read(&p).contains("[jack.first]"));
    }

    #[test]
    fn moving_a_jack_carries_its_comment_and_leaves_the_defaults_behind() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-move", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("patchbay.toml");
        let to = dir.join("spaces/acme.toml");
        std::fs::write(
            &from,
            "[defaults]\nuser = \"root\"\n\n# the old one\n[jack.web]\nhost = \"10.0.0.4\"\nrdp = 3389\n",
        )
        .unwrap();

        move_jack_at(&from, &to, "web").unwrap();
        let landed = read(&to);
        assert!(landed.contains("[jack.web]"), "{landed}");
        assert!(landed.contains("# the old one"), "the comment stayed behind: {landed}");
        // Everything the sheet can't edit comes too, and nothing the source's
        // [defaults] merely lent it does.
        assert!(landed.contains("rdp = 3389"), "{landed}");
        assert!(!landed.contains("user"), "an inherited default was baked in: {landed}");
        assert!(!read(&from).contains("[jack.web]"), "still in the old file");

        // The same name on both sides is refused rather than silently overwritten.
        std::fs::write(&from, "[jack.web]\nhost = \"other\"\n").unwrap();
        assert!(move_jack_at(&from, &to, "web").is_err());
    }

    #[test]
    fn a_space_name_cannot_climb_out_of_the_directory_and_deleting_keeps_a_copy() {
        for bad in ["../evil", "a/b", "sneaky.toml", "", "   "] {
            assert!(space_slug(bad).is_err(), "{bad:?} was allowed as a space name");
        }
        assert_eq!(space_slug("  Acme Ops  ").unwrap(), "Acme Ops");

        let dir = std::env::temp_dir().join(format!("patchbay-{}-spacefiles", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("patchbay.toml");
        create_space_at(&cfg, "acme").unwrap();
        let file = dir.join("spaces/acme.toml");
        assert!(file.exists());
        assert!(create_space_at(&cfg, "acme").is_err(), "made the same space twice");

        std::fs::write(&file, "[jack.web]\nhost = \"10.0.0.4\"\n").unwrap();
        delete_space_at(&cfg, "acme").unwrap();
        assert!(!file.exists());
        assert!(read(&dir.join("spaces/acme.toml.bak")).contains("[jack.web]"));
    }
}
