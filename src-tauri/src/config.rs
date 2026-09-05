//! The only code that writes a config. People hand-edit the file, so every change goes
//! through toml_edit (comments, spacing and key order survive) and lands as a temp
//! file plus rename, so a crash mid-write can't truncate it.

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
    /// What the sheet was opened on (`stamp_of`); a table that hashes differently now was
    /// changed by someone else since.
    pub stamp: Option<String>,
}

/// One number for a table as written, so an edit can say what it was made against. Only
/// ever compared inside one run of the app, so `DefaultHasher` is enough.
pub fn stamp_of(item: &Item) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    item.to_string().hash(&mut h);
    h.finish().to_string()
}

/// The stamp of every table under `[jack]` or `[folder]`, by name. Empty for a file
/// that won't parse: the list will fail on its own, with a better message.
pub fn stamps_at(path: &Path, under: &str) -> std::collections::HashMap<String, String> {
    stamps(&std::fs::read_to_string(path).unwrap_or_default(), under)
}

pub fn stamps(src: &str, under: &str) -> std::collections::HashMap<String, String> {
    let Ok(doc) = src.parse::<DocumentMut>() else {
        return Default::default();
    };
    doc.get(under)
        .and_then(Item::as_table)
        .map(|t| {
            t.iter()
                .map(|(name, item)| (name.to_string(), stamp_of(item)))
                .collect()
        })
        .unwrap_or_default()
}

fn stale(what: &str, stamp: Option<&str>, current: Option<&Item>) -> Result<(), String> {
    let Some(sent) = stamp else { return Ok(()) };
    let now = current.map(stamp_of).unwrap_or_default();
    if sent != now {
        return Err(format!(
            "\"{what}\" was changed by someone else since you opened it. Saving again replaces their change."
        ));
    }
    Ok(())
}

fn read_doc(path: &Path) -> Result<DocumentMut, String> {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        // The first write, whichever sheet it comes from, starts from the template.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FIRST_RUN.into(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    src.parse::<DocumentMut>()
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Our file in `~/.ssh` and the line that makes ssh read it. Relative, because ssh
/// resolves it against `~/.ssh` and an absolute path would bake in this machine's home.
const SSH_FILE: &str = "patchbay.conf";
const SSH_INCLUDE: &str = "Include patchbay.conf";

/// Write the generated host list and make sure `~/.ssh/config` includes it. The user's
/// config is never rewritten: one `Include` line goes in at the top, where a first-wins
/// file needs it, and `to_ssh_config` has already left out every name they define.
pub fn write_ssh_include(dir: &Path, body: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let ours = dir.join(SSH_FILE);
    // Runs on every read of the list, so an unchanged file is not rewritten.
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

/// Remove the generated file and our one `Include` line, leaving everything else as written.
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

/// What a previous install left in `~/.ssh`. The two flags are independent: either the
/// file or the `Include` line can be there without the other.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct Leftovers {
    pub conf_file: bool,
    pub include_line: bool,
    /// Absolute paths, so the window can show what will be removed.
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

/// Matches our `Include` whether written relative or absolute. The whole last path
/// segment must match: `work-patchbay.conf` is someone else's file.
fn is_our_include(line: &str) -> bool {
    let l = line.trim();
    let Some(path) = l
        .strip_prefix("Include ")
        .or_else(|| l.strip_prefix("include "))
    else {
        return false;
    };
    path.trim().rsplit('/').next() == Some(SSH_FILE)
}

fn includes_ours(src: &str) -> bool {
    src.lines().any(is_our_include)
}

/// Temp file plus rename, so the ssh config is never left half-written.
fn write_text(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension("patchbay-tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Same directory, so the rename is atomic.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// What a first run finds when it opens the file by hand: every key, commented out, so
/// the list stays empty until someone means it. `[defaults]` is real and last, with no
/// keys under it: comments with no key after them are trailing decor, and a new table
/// lands above those.
const FIRST_RUN: &str = r#"# patchbay - your devices, one file. Uncomment a block to start.
# https://github.com/pyvnoaim/patchbay
#
# [jack.bastion]
# host = "bastion.example.com"
# port = 2222
# os = "debian"         # picks the icon; the names are under Settings > Appearance
# folders = ["prod"]    # a list: "prod/eu/web" nests, and a device can sit in several
#
# [jack.web]
# host = "10.0.0.4"
# user = "deploy"
# jump = "bastion"      # another device's name, or a raw host; chains follow the jumps
# forward = ["8080:localhost:80"]
# url = "https://10.0.0.4"   # opens as a tab
# rdp = 3389            # or vnc = 5900; remote desktop through the jump chain
# desc = "what someone arriving here should know"

# Inherited by every device that doesn't set its own: user = "root", key = "~/.ssh/id_ed25519".
[defaults]
"#;

/// The desktop opener does nothing for a missing path, so a first run gets the
/// commented template rather than a blank page.
pub fn ensure_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    write_doc(path, &read_doc(path)?)
}

/// Remove a table and hand back the comments above it for `rehome_comments`: toml_edit
/// would otherwise delete them, file header included.
/// ponytail: the whole block moves, so a comment about a deleted jack lands above the
/// next one. Stale is visible and fixable; deleted is not.
pub fn orphan_comments(parent: &mut Table, key: &str) -> Option<(String, usize)> {
    let removed = parent.remove(key)?;
    let t = removed.as_table()?;
    let prefix = t.decor().prefix()?.as_str()?;
    if prefix.trim().is_empty() {
        return None;
    }
    Some((prefix.to_string(), t.position()?))
}

/// Tables render in `position()` order, so the next position along is the one that
/// takes the removed table's place, wherever it lives in the tree.
fn first_position_after(item: &Item, after: usize) -> Option<usize> {
    let t = item.as_table()?;
    t.iter()
        .filter_map(|(_, v)| first_position_after(v, after))
        .chain(t.position().filter(|p| *p > after))
        .min()
}

fn prepend_prefix(item: &mut Item, at: usize, comments: &str) -> bool {
    let Some(t) = item.as_table_mut() else {
        return false;
    };
    if t.position() == Some(at) {
        let old = t
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        t.decor_mut().set_prefix(format!("{comments}{old}"));
        return true;
    }
    t.iter_mut().any(|(_, v)| prepend_prefix(v, at, comments))
}

pub fn rehome_comments(doc: &mut DocumentMut, orphan: Option<(String, usize)>) {
    let Some((comments, was_at)) = orphan else {
        return;
    };
    match first_position_after(doc.as_item(), was_at) {
        Some(at) => {
            prepend_prefix(doc.as_item_mut(), at, &comments);
        }
        // Nothing renders after it, so the comments end the file.
        None => {
            let trailing = doc.trailing().as_str().unwrap_or("").to_string();
            doc.set_trailing(format!("{comments}{trailing}"));
        }
    }
}

/// `[jack]` stays implicit: only the `[jack.name]` children are written.
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
    let kept: Vec<&str> = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
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

/// `original` is None when adding and the old name when editing; a different name renames.
pub fn save_jack_at(path: &Path, original: Option<String>, j: JackInput) -> Result<(), String> {
    let name = j.name.trim().to_string();
    if name.is_empty() {
        return Err("a jack needs a name".into());
    }
    if j.host.trim().is_empty() {
        return Err(format!("\"{name}\" needs a host"));
    }
    if let Some(u) = j.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        if !patchbay::is_web_url(u) {
            return Err("a url has to start with http:// or https://".into());
        }
    }
    // A forward becomes argv, so it is validated on save as well as on connect.
    for f in &j.forward {
        crate::patchbay::forward_arg(f)?;
    }

    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;
    if let Some(o) = original.as_deref() {
        stale(o, j.stamp.as_deref(), jacks.get(o))?;
    }

    let renaming = original.as_deref().is_some_and(|o| o != name);
    if (original.is_none() || renaming) && jacks.contains_key(&name) {
        return Err(format!("there's already a jack named \"{name}\""));
    }
    // A rename keeps the jack's comments, position and any key the sheet can't edit.
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
    set_arr(t, "forward", &j.forward);
    // Only written when false, to keep configs uncluttered.
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

/// Set or clear a folder's note. A blank note removes it, and the `[folder]` table goes
/// with the last one.
pub fn set_note_at(
    file: &Path,
    folder: &str,
    note: &str,
    stamp: Option<&str>,
) -> Result<(), String> {
    let path = folder.trim().trim_matches('/');
    if path.is_empty() {
        return Err("a note belongs to a folder".into());
    }
    let mut doc = read_doc(file)?;
    stale(path, stamp, doc.get("folder").and_then(|f| f.get(path)))?;
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

/// What a delete took out, as the window holds it for Undo: the table as TOML, and
/// where it sat, so putting it back lands it in the same place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Removed {
    pub name: String,
    pub block: String,
    pub position: Option<usize>,
}

/// Hands back the removed jack so a wrong "yes" can be undone. The comments above it
/// go to the next table (`rehome_comments`) and stay there through an undo: a comment
/// left where it was is visible and fixable, a duplicated one is not.
pub fn delete_jack_at(path: &Path, name: &str) -> Result<Removed, String> {
    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;
    let Some(item) = jacks.get(name).cloned() else {
        return Err(format!("no jack named \"{name}\""));
    };
    let position = item.as_table().and_then(Table::position);
    let orphan = orphan_comments(jacks, name);
    rehome_comments(&mut doc, orphan);
    write_doc(path, &doc)?;

    // A one-table document, so the block round-trips through the same parser.
    let mut alone = DocumentMut::new();
    let mut only = Table::new();
    only.set_implicit(true);
    only[name] = item;
    if let Some(t) = only[name].as_table_mut() {
        t.decor_mut().set_prefix("");
    }
    alone["jack"] = Item::Table(only);
    Ok(Removed {
        name: name.to_string(),
        block: alone.to_string(),
        position,
    })
}

/// Undo: the removed table goes back in, at the position it had, unless the name has
/// been taken since.
pub fn restore_jack_at(path: &Path, removed: &Removed) -> Result<(), String> {
    let alone = removed
        .block
        .parse::<DocumentMut>()
        .map_err(|e| format!("could not restore \"{}\": {e}", removed.name))?;
    let Some(item) = alone
        .get("jack")
        .and_then(Item::as_table)
        .and_then(|t| t.get(&removed.name))
        .cloned()
    else {
        return Err(format!("could not restore \"{}\"", removed.name));
    };
    let mut doc = read_doc(path)?;
    if jack_table(&mut doc)?.contains_key(&removed.name) {
        return Err(format!("there's already a jack named \"{}\"", removed.name));
    }
    // Positions are renumbered by every parse, so the slot it had is taken by whatever
    // followed it: everything from there on moves down one to make room.
    let at = removed.position.unwrap_or(usize::MAX);
    make_room(doc.as_item_mut(), at);
    let jacks = jack_table(&mut doc)?;
    jacks[&removed.name] = item;
    if let Some(t) = jacks[&removed.name].as_table_mut() {
        t.set_position(at);
        if t.decor()
            .prefix()
            .is_none_or(|p| p.as_str().unwrap_or("").is_empty())
        {
            t.decor_mut().set_prefix("\n");
        }
    }
    write_doc(path, &doc)
}

fn make_room(item: &mut Item, at: usize) {
    let Some(t) = item.as_table_mut() else {
        return;
    };
    if let Some(p) = t.position().filter(|p| *p >= at) {
        t.set_position(p + 1);
    }
    for (_, v) in t.iter_mut() {
        make_room(v, at);
    }
}

/// Only the `folders` list, for a drag into a folder: the rest of the table, comments
/// and the keys the sheet doesn't know included, is left exactly as written.
pub fn set_folders_at(path: &Path, name: &str, folders: &[String]) -> Result<(), String> {
    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;
    let t = jacks
        .get_mut(name)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    set_arr(t, "folders", folders);
    write_doc(path, &doc)
}

/// App preferences, in `[settings]`. Defaults are what you get with no section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Settings {
    /// TCP-probe every device's entry point on a timer, for the status dots.
    #[serde(default = "yes")]
    pub probe: bool,
    /// Connect opens the system terminal instead of a tab in the window.
    #[serde(default)]
    pub connect_in_terminal: bool,
    /// Write the list to `~/.ssh/patchbay.conf` and include it from `~/.ssh/config`.
    /// Off by default: it writes outside patchbay's own directory.
    #[serde(default)]
    pub write_ssh_config: bool,
    /// Tint a device's icon by its `os`, using the brand colour unless `[colors]` overrides it.
    #[serde(default = "yes")]
    pub os_colors: bool,
    /// Ask the update endpoint once per launch. The only request the app makes on its own.
    #[serde(default = "yes")]
    pub check_updates: bool,
    /// Append each session's output to a file under `logs/` beside the config. Off by default.
    #[serde(default)]
    pub log_sessions: bool,
    /// "system", "light" or "dark". Anything else reads as "system".
    #[serde(default = "system")]
    pub theme: String,
    /// Terminal font size in px. Clamped on save; it reaches xterm unchecked.
    #[serde(default = "font_size")]
    pub font_size: f64,
    /// Sidebar width in px. Clamped on save; a column wider than the window leaves no list.
    #[serde(default = "sidebar")]
    pub sidebar: f64,
    /// A shared list somewhere else: the devices, `[defaults]` and folder notes are read
    /// and written there instead. Everything else in this file stays this machine's.
    #[serde(default)]
    pub list: Option<String>,
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
            list: None,
        }
    }
}

/// The four `[defaults]` keys the sheet edits. Any other key written by hand is left alone.
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

/// Never fails: the sheet has to open even when the file it is about to fix is broken.
pub fn load_defaults_at(file: &Path) -> Defaults {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| toml::from_str::<RawDefaults>(&s).ok())
        .map(|r| r.defaults)
        .unwrap_or_default()
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
    // An empty `[defaults]` is removed; a hand-written key keeps the table alive.
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

/// Never fails: a broken or missing config means defaults, so the sheet still opens.
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
    t["write_ssh_config"] = value(s.write_ssh_config);
    // Read straight back by the window (root element, xterm), so narrowed here.
    t["theme"] = value(match s.theme.as_str() {
        "light" => "light",
        "dark" => "dark",
        _ => "system",
    });
    t["font_size"] = value(s.font_size.clamp(8.0, 32.0));
    t["sidebar"] = value(s.sidebar.clamp(150.0, 480.0));
    set_str(t, "list", s.list.as_deref());
    write_doc(file, &doc)
}

/// The first copy of a shared list: this machine's devices, `[defaults]` and folder
/// notes, without `[settings]` and `[colors]`, which are this machine's. Refuses to
/// replace a file that is there. The one write allowed at a path that isn't, and even
/// so never into a directory that isn't: a missing directory is a share that is away,
/// and one made at `/Volumes/team` is where the share would have mounted next time.
pub fn seed_list_at(own: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        return Err(format!("{}: already there", to.display()));
    }
    if !to.parent().is_some_and(Path::is_dir) {
        return Err(format!("{}: that folder is not there", to.display()));
    }
    let mut doc = read_doc(own)?;
    for key in ["settings", "colors"] {
        let orphan = orphan_comments(doc.as_table_mut(), key);
        rehome_comments(&mut doc, orphan);
    }
    write_doc(to, &doc)
}

#[derive(Debug, Default, Deserialize)]
struct RawColors {
    #[serde(default)]
    colors: std::collections::BTreeMap<String, String>,
}

/// `[colors]` maps an `os` value to a hex. Only deliberate overrides are stored.
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
        // Reaches a style attribute, so only a plain hex gets in.
        let ok =
            h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit());
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

/// A note hangs on a path, so a renamed folder takes it along and a deleted one drops it.
fn move_note(doc: &mut DocumentMut, from: &str, to: Option<&str>) {
    let Some(table) = doc.get_mut("folder").and_then(Item::as_table_mut) else {
        return;
    };
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
        let Some(t) = item.as_table_mut() else {
            continue;
        };
        let Some(arr) = t.get("folders").and_then(|i| i.as_array()) else {
            continue;
        };

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
                t.remove("folders");
            } else {
                t["folders"] = value(next);
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

/// Fold the old `spaces/*.toml` files into the one list. Each device gets the space's
/// name as its outermost folder, so `acme` + `prod` becomes `acme/prod`. The files read
/// are renamed `.toml.merged`, not deleted: they are the only copy of the split version.
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
        let Some(space) = file
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(from) = src.parse::<DocumentMut>() else {
            // A file that doesn't parse is left where it is and named in the log.
            eprintln!(
                "patchbay: {} doesn't parse, so it was left alone",
                file.display()
            );
            continue;
        };
        let Some(jacks) = from.get("jack").and_then(Item::as_table) else {
            continue;
        };
        // A space's own `[defaults]` is written onto each of its devices, since there
        // is only one `[defaults]` after this.
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
            // The space becomes the outermost folder.
            let mut folders = Array::new();
            match t.get("folders").and_then(Item::as_array) {
                Some(had) if !had.is_empty() => {
                    for v in had.iter().filter_map(|v| v.as_str()) {
                        folders.push(format!("{space}/{v}"));
                    }
                }
                _ => folders.push(space.clone()),
            }
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
            // First wins: a name already in the list is the one in use.
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
    // Written before renaming: the other order loses the devices if the write fails.
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
            name: name.into(),
            host: host.into(),
            user: None,
            port: None,
            key: None,
            jump: None,
            os: None,
            url: None,
            rdp: None,
            vnc: None,
            ssh: None,
            primary: None,
            desc: None,
            folders: vec![],
            forward: vec![],
            stamp: None,
        }
    }

    #[test]
    fn a_theme_and_a_font_size_are_narrowed_on_the_way_in() {
        let p = scratch("settings");
        save_settings_at(
            &p,
            &Settings {
                theme: "neon".into(),
                font_size: 900.0,
                ..Settings::default()
            },
        )
        .unwrap();
        let back = load_settings_at(&p);
        assert_eq!(back.theme, "system");
        assert_eq!(back.font_size, 32.0);

        save_settings_at(
            &p,
            &Settings {
                theme: "light".into(),
                font_size: 14.0,
                ..Settings::default()
            },
        )
        .unwrap();
        let back = load_settings_at(&p);
        assert_eq!(back.theme, "light");
        assert_eq!(back.font_size, 14.0);
        assert!(read(&p).contains("keep this comment"));
    }

    #[test]
    fn the_update_check_is_on_until_it_is_turned_off() {
        let p = scratch("check_updates");
        // Absent is on: a config written before the setting existed still checks.
        assert!(load_settings_at(&p).check_updates);

        save_settings_at(
            &p,
            &Settings {
                check_updates: false,
                ..Settings::default()
            },
        )
        .unwrap();
        assert!(!load_settings_at(&p).check_updates);
    }

    #[test]
    fn session_logging_is_off_until_it_is_turned_on() {
        let p = scratch("log_sessions");
        assert!(!load_settings_at(&p).log_sessions);

        save_settings_at(
            &p,
            &Settings {
                log_sessions: true,
                ..Settings::default()
            },
        )
        .unwrap();
        assert!(load_settings_at(&p).log_sessions);
    }

    #[test]
    fn the_ssh_config_switch_survives_a_save() {
        let p = scratch("write_ssh_config");
        save_settings_at(
            &p,
            &Settings {
                write_ssh_config: true,
                ..Settings::default()
            },
        )
        .unwrap();
        assert!(load_settings_at(&p).write_ssh_config);
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

        // Clearing every field takes the section with it.
        save_defaults_at(&p, &Defaults::default()).unwrap();
        assert!(!read(&p).contains("[defaults]"), "got {}", read(&p));
    }

    #[test]
    fn comments_outlive_the_table_they_sat_above() {
        let p = scratch("comments");
        save_defaults_at(&p, &Defaults::default()).unwrap();
        let out = read(&p);
        assert!(!out.contains("[defaults]"), "got {out}");
        assert!(
            out.contains("# my hosts"),
            "the file header went with it:\n{out}"
        );
        assert!(
            out.find("# my hosts") < out.find("[jack.bastion]"),
            "the header should still be on top:\n{out}"
        );

        delete_jack_at(&p, "bastion").unwrap();
        let out = read(&p);
        assert!(out.contains("# my hosts"), "got {out}");
        assert!(out.contains("# the way in"), "got {out}");
        assert!(out.find("# my hosts") < out.find("[jack.web]"), "got {out}");

        // Nothing renders after the last jack, so its comments end the file.
        delete_jack_at(&p, "web").unwrap();
        assert!(read(&p).contains("# my hosts"), "got {}", read(&p));
    }

    #[test]
    fn renaming_a_jack_takes_its_comment_along() {
        let p = scratch("rename-comment");
        save_jack_at(
            &p,
            Some("bastion".into()),
            input("gateway", "bastion.example"),
        )
        .unwrap();
        let out = read(&p);
        assert!(out.contains("# the way in"), "got {out}");
        assert!(
            out.find("# the way in") < out.find("[jack.gateway]"),
            "got {out}"
        );
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
        assert!(
            !out.contains("port = 2222"),
            "cleared port should be gone:\n{out}"
        );
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
        assert!(save_jack_at(&p, None, input("", "h"))
            .unwrap_err()
            .contains("needs a name"));
        assert!(save_jack_at(&p, None, input("x", "  "))
            .unwrap_err()
            .contains("needs a host"));
    }

    /// Every device arrives, and the split survives as a branch of the tree.
    #[test]
    fn spaces_fold_into_the_one_list_and_keep_their_shape() {
        let cfg = std::env::temp_dir().join(format!(
            "patchbay-{}-fold/patchbay.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
        std::fs::create_dir_all(cfg.with_file_name("spaces")).unwrap();
        std::fs::write(&cfg, "# mine\n[jack.laptop]\nhost = \"192.168.1.9\"\n").unwrap();
        std::fs::write(
            cfg.with_file_name("spaces").join("acme.toml"),
            "[defaults]\nuser = \"root\"\n\n[jack.db]\nhost = \"10.0.0.5\"\n\
             [jack.web]\nhost = \"10.0.0.4\"\nuser = \"deploy\"\nfolders = [\"prod\"]\n",
        )
        .unwrap();
        // A name already in the main list.
        std::fs::write(
            cfg.with_file_name("spaces").join("lab.toml"),
            "[jack.laptop]\nhost = \"10.1.1.1\"\n",
        )
        .unwrap();

        assert_eq!(fold_spaces_at(&cfg).unwrap(), 3);
        let jacks = patchbay::parse(&read(&cfg)).unwrap();
        assert_eq!(jacks.len(), 4);
        assert!(read(&cfg).contains("# mine"), "the file was re-serialized");

        // Inherited keys come along; keys set on the device itself win.
        assert_eq!(
            jacks["db"].user.as_deref(),
            Some("root"),
            "an inherited user was dropped"
        );
        assert_eq!(
            jacks["web"].user.as_deref(),
            Some("deploy"),
            "an inherited user won"
        );

        // The space is the outermost folder, with or without folders before.
        assert_eq!(
            jacks["db"].folders.as_deref(),
            Some(&["acme".to_string()][..])
        );
        assert_eq!(
            jacks["web"].folders.as_deref(),
            Some(&["acme/prod".to_string()][..])
        );
        // The existing device is untouched; the clash gets its own name.
        assert_eq!(jacks["laptop"].host, "192.168.1.9");
        assert_eq!(jacks["laptop 2"].host, "10.1.1.1");

        // The files it read are kept, and it does not run twice.
        assert!(cfg
            .with_file_name("spaces")
            .join("acme.toml.merged")
            .exists());
        assert_eq!(fold_spaces_at(&cfg).unwrap(), 0);
    }

    #[test]
    fn a_folder_note_survives_a_rename_and_goes_with_a_delete() {
        let p = scratch("notes");
        set_note_at(
            &p,
            "prod/eu",
            "the recovery key is in the safe\nask Anna first",
            None,
        )
        .unwrap();
        assert_eq!(
            patchbay::notes(&read(&p))["prod/eu"],
            "the recovery key is in the safe\nask Anna first",
            "a note has to survive a round trip with its line breaks"
        );

        rename_group_at(&p, "prod/eu", "prod/emea").unwrap();
        let after = patchbay::notes(&read(&p));
        assert!(
            after.contains_key("prod/emea"),
            "the note was orphaned by a rename"
        );
        assert!(!after.contains_key("prod/eu"));

        delete_group_at(&p, "prod/emea").unwrap();
        assert!(patchbay::notes(&read(&p)).is_empty());
        assert!(
            !read(&p).contains("[folder"),
            "an empty table was left behind"
        );

        // A blank note is a removal.
        set_note_at(&p, "prod", "x", None).unwrap();
        set_note_at(&p, "prod", "  ", None).unwrap();
        assert!(patchbay::notes(&read(&p)).is_empty());
        assert!(
            read(&p).contains("keep this comment"),
            "the file was re-serialized"
        );
    }

    #[test]
    fn renaming_a_group_rewrites_the_whole_subtree() {
        let p = scratch("group-rename");
        assert_eq!(rename_group_at(&p, "prod/eu", "prod/emea").unwrap(), 2);
        let out = read(&p);
        assert!(out.contains(r#""prod/emea""#), "{out}");
        assert!(
            out.contains(r#""prod/emea/web""#),
            "children move too:\n{out}"
        );
        assert!(
            out.contains(r#""entrypoint""#),
            "unrelated folders untouched"
        );
    }

    #[test]
    fn deleting_a_group_drops_the_folders_but_keeps_the_jacks() {
        let p = scratch("group-delete");
        assert_eq!(delete_group_at(&p, "prod/eu").unwrap(), 2);
        let out = read(&p);
        assert!(!out.contains("prod/eu"));
        assert!(out.contains("[jack.bastion]"), "the device stays");
        assert!(out.contains("[jack.web]"), "the device stays");
        assert!(
            out.contains(r#"folders = ["entrypoint"]"#),
            "its other folder stays"
        );
        // web's only folder was under it, so the key goes entirely.
        assert!(!out.contains(r#"folders = []"#));
    }

    #[test]
    fn colours_are_validated_and_clearable() {
        let p = scratch("colors");
        save_color_at(&p, "Synology", Some("#0C4A9F")).unwrap();
        let out = read(&p);
        assert!(out.contains("[colors]"), "{out}");
        assert!(
            out.contains(r##"synology = "#0C4A9F""##),
            "key is lowercased: {out}"
        );

        for bad in ["blue", "#0C4A9", "#GGGGGG", "red; background:url(x)"] {
            assert!(
                save_color_at(&p, "x", Some(bad)).is_err(),
                "{bad:?} should be rejected"
            );
        }

        save_color_at(&p, "synology", None).unwrap();
        assert!(!read(&p).contains("synology ="));
    }

    #[test]
    fn deleting_a_jack_reports_an_unknown_name() {
        let p = scratch("delete");
        delete_jack_at(&p, "web").unwrap();
        assert!(!read(&p).contains("[jack.web]"));
        assert!(delete_jack_at(&p, "web")
            .unwrap_err()
            .contains("no jack named"));
    }

    /// Undo puts the table back where it was, keys and order intact, and refuses once
    /// the name is in use again.
    #[test]
    fn a_deleted_jack_comes_back_in_its_place() {
        let p = scratch("undo");
        let removed = delete_jack_at(&p, "bastion").unwrap();
        assert_eq!(removed.name, "bastion");
        assert!(removed.block.contains("[jack.bastion]"));
        assert!(removed.block.contains("port = 2222"));
        assert!(!read(&p).contains("[jack.bastion]"));

        restore_jack_at(&p, &removed).unwrap();
        let back = read(&p);
        assert!(back.contains("[jack.bastion]\nhost = \"bastion.example\"\nport = 2222"));
        assert!(
            back.find("[jack.bastion]").unwrap() < back.find("[jack.web]").unwrap(),
            "restored ahead of the table that followed it"
        );
        assert!(back.contains("# my hosts - keep this comment"));
        assert!(
            back.matches("# the way in").count() == 1,
            "the comment above it was rehomed once, not duplicated"
        );
        assert!(restore_jack_at(&p, &removed)
            .unwrap_err()
            .contains("already a jack named"));
    }

    #[test]
    fn a_folder_move_touches_nothing_but_folders() {
        let p = scratch("move");
        set_folders_at(&p, "web", &["staging/web".into()]).unwrap();
        let s = read(&p);
        assert!(s.contains("folders = [\"staging/web\"]"));
        assert!(s.contains("jump = \"bastion\""), "the other keys stay");
        assert!(s.contains("# trailing comment"));
        set_folders_at(&p, "web", &[]).unwrap();
        assert!(!read(&p).contains("[jack.web]\nhost = \"10.0.0.4\"\njump = \"bastion\"\nfolders"));
        assert!(set_folders_at(&p, "nope", &[]).is_err());
    }

    /// One line goes in at the top; turning it off takes exactly that line and the file.
    #[test]
    fn the_ssh_include_is_one_line_of_theirs_and_comes_back_out_cleanly() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-sshinc", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        let mine = "Host mine\n  HostName 10.0.0.2\n";
        std::fs::write(&cfg, mine).unwrap();

        write_ssh_include(&dir, "Host db\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("patchbay.conf")).unwrap(),
            "Host db\n"
        );
        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.starts_with("Include patchbay.conf\n"),
            "at the top: {after:?}"
        );
        assert!(after.contains(mine), "everything they wrote is still there");

        // Idempotent: this runs on every read of the list.
        write_ssh_include(&dir, "Host db\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            after,
            "no second Include"
        );

        remove_ssh_include(&dir).unwrap();
        assert!(!dir.join("patchbay.conf").exists());
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            mine,
            "theirs, untouched"
        );
    }

    /// Only our own Include is removed, never a similarly named one of theirs.
    #[test]
    fn only_our_own_include_line_is_ours_to_remove() {
        assert!(is_our_include("Include patchbay.conf"));
        assert!(is_our_include("  include ~/.ssh/patchbay.conf  "));
        assert!(!is_our_include("Include ~/.ssh/work-patchbay.conf"));
        assert!(!is_our_include("Include conf.d/*.conf"));
        assert!(!is_our_include("Host patchbay.conf"));
    }

    #[test]
    fn a_machine_with_no_ssh_config_gets_one_with_only_the_include_in_it() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-sshnew", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        write_ssh_include(&dir, "Host db\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config")).unwrap(),
            "Include patchbay.conf\n\n"
        );
        remove_ssh_include(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config")).unwrap().trim(),
            ""
        );
    }

    /// Uninstall is drag-to-trash on macOS, so a previous install can leave both behind.
    #[test]
    fn a_previous_install_can_be_swept_up_and_a_clean_machine_says_so() {
        let dir = std::env::temp_dir().join(format!("patchbay-{}-sshleft", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let l = ssh_leftovers(&dir);
        assert!(!l.conf_file && !l.include_line);
        write_ssh_include(&dir, "Host db\n").unwrap();
        let l = ssh_leftovers(&dir);
        assert!(
            l.conf_file && l.include_line,
            "the file and the include line are both here"
        );
        assert!(l.conf_path.ends_with("patchbay.conf"));
        // File deleted by hand, line left behind: still a leftover.
        std::fs::remove_file(dir.join(SSH_FILE)).unwrap();
        let l = ssh_leftovers(&dir);
        assert!(!l.conf_file && l.include_line);
        remove_ssh_include(&dir).unwrap();
        let l = ssh_leftovers(&dir);
        assert!(!l.conf_file && !l.include_line);
    }

    #[test]
    fn a_missing_config_is_created_rather_than_erroring() {
        let p = std::env::temp_dir().join(format!(
            "patchbay-{}-fresh/patchbay.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        save_jack_at(&p, None, input("first", "10.0.0.1")).unwrap();
        let s = read(&p);
        assert!(s.starts_with("# patchbay - your devices"), "got {s}");
        assert!(s.contains("[jack.first]"));
    }

    #[test]
    fn first_run_template_is_empty_and_survives_the_first_save() {
        let p = std::env::temp_dir().join(format!("patchbay-{}-firstrun.toml", std::process::id()));
        let _ = std::fs::remove_file(&p);
        ensure_exists(&p).unwrap();
        assert!(patchbay::load(&p).unwrap().is_empty());
        save_jack_at(&p, None, input("web", "10.0.0.4")).unwrap();
        let s = read(&p);
        assert!(s.starts_with("# patchbay - your devices"));
        assert!(s.contains("# rdp = 3389"));
        assert!(
            s.ends_with("[defaults]\n\n[jack.web]\nhost = \"10.0.0.4\"\n"),
            "got {s}"
        );
        assert_eq!(patchbay::load(&p).unwrap().len(), 1);
        ensure_exists(&p).unwrap(); // a second call leaves the file alone
        assert_eq!(read(&p), s);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn an_edit_made_against_a_stale_table_is_refused_and_the_stamp_moves() {
        let p = scratch("stamp");
        let before = stamps_at(&p, "jack")["web"].clone();
        // A colleague's edit lands between opening the sheet and saving.
        set_folders_at(&p, "web", &["theirs".into()]).unwrap();
        let mut mine = input("web", "10.0.0.9");
        mine.stamp = Some(before);
        let err = save_jack_at(&p, Some("web".into()), mine).unwrap_err();
        assert!(err.contains("changed by someone else"), "{err}");
        assert!(read(&p).contains("theirs"), "the refusal writes nothing");
        // Reloaded: the new stamp goes through and knowingly replaces theirs.
        let mut again = input("web", "10.0.0.9");
        again.stamp = Some(stamps_at(&p, "jack")["web"].clone());
        save_jack_at(&p, Some("web".into()), again).unwrap();
        assert!(read(&p).contains("10.0.0.9"));
        // No stamp is no check: a drag, an import, an old window.
        save_jack_at(&p, Some("web".into()), input("web", "10.0.0.10")).unwrap();
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn a_note_saved_over_someone_elses_is_refused() {
        let p = scratch("note-stamp");
        set_note_at(&p, "prod", "mine", None).unwrap();
        let mine = stamps_at(&p, "folder")["prod"].clone();
        set_note_at(&p, "prod", "theirs", None).unwrap();
        assert!(set_note_at(&p, "prod", "mine again", Some(&mine)).is_err());
        assert!(patchbay::notes(&read(&p))["prod"] == "theirs");
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn a_seed_carries_the_list_and_leaves_this_machines_settings_behind() {
        let own = scratch("seed-own");
        save_settings_at(
            &own,
            &Settings {
                list: Some("/somewhere/else.toml".into()),
                ..Settings::default()
            },
        )
        .unwrap();
        save_color_at(&own, "debian", Some("#112233")).unwrap();
        set_note_at(&own, "prod", "careful", None).unwrap();
        let to = own.with_file_name("patchbay-seeded.toml");
        let _ = std::fs::remove_file(&to);
        seed_list_at(&own, &to).unwrap();
        let seeded = read(&to);
        assert!(seeded.contains("[jack.web]"));
        assert!(seeded.contains("careful"));
        assert!(!seeded.contains("[settings]"), "{seeded}");
        assert!(!seeded.contains("#112233"), "{seeded}");
        assert!(read(&own).contains("[jack.web]"), "the seed is a copy");
        assert!(
            seed_list_at(&own, &to).is_err(),
            "never over a file that is there"
        );
        // A missing directory is a share that is away, not something to create.
        let gone = std::env::temp_dir()
            .join(format!("patchbay-nowhere-{}", std::process::id()))
            .join("patchbay.toml");
        let err = seed_list_at(&own, &gone).unwrap_err();
        assert!(err.contains("not there"), "{err}");
        assert!(!gone.parent().unwrap().exists());
        std::fs::remove_file(&own).unwrap();
        std::fs::remove_file(&to).unwrap();
    }

    #[test]
    fn the_list_setting_round_trips_and_clears() {
        let p = scratch("list-setting");
        let mut s = Settings {
            list: Some("~/team/patchbay.toml".into()),
            ..Settings::default()
        };
        save_settings_at(&p, &s).unwrap();
        assert_eq!(
            load_settings_at(&p).list.as_deref(),
            Some("~/team/patchbay.toml")
        );
        s.list = None;
        save_settings_at(&p, &s).unwrap();
        assert_eq!(load_settings_at(&p).list, None);
        assert!(!read(&p).contains("list"));
        std::fs::remove_file(&p).unwrap();
    }
}
