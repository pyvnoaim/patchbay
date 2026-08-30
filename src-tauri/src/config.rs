//! Writing the config back out. The file is something people hand-edit, so every
//! change goes through toml_edit — comments, spacing and key order survive — and
//! lands via a temp file + rename so a crash mid-write can't truncate it.

use crate::patchbay;
use serde::Deserialize;
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
    pub desc: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
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

    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;

    let renaming = original.as_deref().is_some_and(|o| o != name);
    if (original.is_none() || renaming) && jacks.contains_key(&name) {
        return Err(format!("there's already a jack named \"{name}\""));
    }
    if renaming {
        jacks.remove(original.as_deref().unwrap_or_default());
    }

    let entry = jacks
        .entry(&name)
        .or_insert_with(|| Item::Table(Table::new()));
    let t = entry
        .as_table_mut()
        .ok_or_else(|| format!("[jack.{name}] isn't a table"))?;

    set_str(t, "host", Some(&j.host));
    set_str(t, "user", j.user.as_deref());
    set_str(t, "key", j.key.as_deref());
    set_str(t, "jump", j.jump.as_deref());
    set_str(t, "os", j.os.as_deref());
    set_str(t, "desc", j.desc.as_deref());
    set_arr(t, "tags", &j.tags);
    set_arr(t, "forward", &j.forward);
    match j.port {
        Some(p) => t["port"] = value(p as i64),
        None => {
            t.remove("port");
        }
    }

    write_doc(path, &doc)
}

pub fn delete_jack(name: &str) -> Result<(), String> {
    delete_jack_at(&patchbay::config_path(), name)
}

pub fn delete_jack_at(path: &Path, name: &str) -> Result<(), String> {
    let mut doc = read_doc(path)?;
    let jacks = jack_table(&mut doc)?;
    if jacks.remove(name).is_none() {
        return Err(format!("no jack named \"{name}\""));
    }
    write_doc(path, &doc)
}

/// Rewrite every tag that is `path` or sits under `path/`. `to` of None deletes them.
/// This is what folder rename/delete means when folders are just slash-delimited tags.
fn map_tags(file: &Path, path: &str, to: Option<&str>) -> Result<usize, String> {
    let mut doc = read_doc(file)?;
    let jacks = jack_table(&mut doc)?;
    let mut touched = 0;

    for (_, item) in jacks.iter_mut() {
        let Some(t) = item.as_table_mut() else { continue };
        let Some(arr) = t.get("tags").and_then(|i| i.as_array()) else { continue };

        let mut next = Array::new();
        let mut changed = false;
        for tag in arr.iter().filter_map(|v| v.as_str()) {
            let under = tag == path || tag.starts_with(&format!("{path}/"));
            if !under {
                next.push(tag);
                continue;
            }
            changed = true;
            if let Some(to) = to {
                next.push(format!("{to}{}", &tag[path.len()..]).as_str());
            }
        }
        if changed {
            touched += 1;
            if next.is_empty() {
                t.remove("tags");
            } else {
                t["tags"] = value(next);
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
    map_tags(file, from, Some(to))
}

pub fn delete_group(path: &str) -> Result<usize, String> {
    delete_group_at(&patchbay::config_path(), path)
}

pub fn delete_group_at(file: &Path, path: &str) -> Result<usize, String> {
    map_tags(file, path, None)
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
tags = ["prod/eu", "entrypoint"]

[jack.web]
host = "10.0.0.4"
jump = "bastion"
tags = ["prod/eu/web"]
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
            user: None, port: None, key: None, jump: None, os: None, desc: None,
            tags: vec![], forward: vec![],
        }
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
        j.tags = vec!["prod/eu".into()];
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
        assert!(out.contains(r#""entrypoint""#), "unrelated tags untouched");
    }

    #[test]
    fn deleting_a_group_drops_the_tags_but_keeps_the_jacks() {
        let p = scratch("group-delete");
        assert_eq!(delete_group_at(&p, "prod/eu").unwrap(), 2);
        let out = read(&p);
        assert!(!out.contains("prod/eu"));
        assert!(out.contains("[jack.bastion]"), "the device stays");
        assert!(out.contains("[jack.web]"), "the device stays");
        assert!(out.contains(r#"tags = ["entrypoint"]"#), "its other tag stays");
        // web's only tag was under the folder, so the key goes entirely
        assert!(!out.contains(r#"tags = []"#));
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
