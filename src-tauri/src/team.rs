//! Team sync: one *space* - one config file in `spaces/` - mirrored through the team
//! server. Your own list is the main config and never goes anywhere.
//!
//! The space's whole document is the shared thing. The server stores a string and
//! never parses it, so there is no schema on that side and nothing to merge here -
//! the two ends only ever agree or disagree.
//!
//! The transport is plain HTTP: `GET` hands back the document with an `ETag`, `PUT`
//! sends it with `If-Match`. That is how every HTTP file store spells optimistic
//! concurrency, so a space can point at one instead of at `server/` - and a space
//! with no code is a read-only subscription to whatever is at that URL.
//!
//! Our half lives in `team.toml` *beside* the config, never in a space, because a
//! space file is what gets uploaded: a team code in there would be a credential in a
//! file the whole team reads, and the device id would stop counting seats the moment
//! it was shared. Nothing is stripped on the way out any more - a team space holds
//! the team's devices and nothing else, which is what makes joining one safe.

use crate::{config, patchbay};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use toml_edit::DocumentMut;

/// Long enough for a laptop on hotel wifi, short enough that a dead server doesn't
/// hold the window's focus handler.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Two syncs overlapping - the window regaining focus while an edit finishes - would
/// race their own puts, and the loser's 409 reads as a conflict the user never had.
/// They are cheap and idempotent, so the second one just waits and then sees the
/// first one's answer.
static RUNNING: Mutex<()> = Mutex::new(());

/// One space's half of the arrangement. `device` is not in here: it is this machine,
/// not this team, and one seat per machine is what the seat count means.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Team {
    pub url: String,
    /// Absent means nobody is being asked for one: a plain URL, read-only, no writes
    /// attempted and none expected to be allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The `ETag` and document hash we last agreed on. Between them they say who
    /// moved: a different hash means we edited, a different tag means they did, and
    /// both at once is the one case nobody can merge for us.
    #[serde(default)]
    pub etag: String,
    #[serde(default)]
    pub synced: String,
    /// Which space this is, filled in on load. Never written - the table name is it.
    #[serde(skip)]
    pub space: String,
}

/// The whole of `team.toml`: this machine's id, and one table per team space.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Teams {
    #[serde(default)]
    device: String,
    #[serde(default)]
    space: std::collections::BTreeMap<String, Team>,
}

/// Beside the config, and derived from it, so `$PATCHBAY_CONFIG` moves both together
/// and the tests get a scratch team the same way they get a scratch config.
fn team_path(cfg: &Path) -> PathBuf {
    cfg.with_file_name("team.toml")
}

fn load_teams(cfg: &Path) -> Teams {
    let mut t: Teams = std::fs::read_to_string(team_path(cfg))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    t.space.retain(|_, v| !v.url.is_empty());
    for (name, v) in t.space.iter_mut() {
        v.space = name.clone();
    }
    t
}

fn load(cfg: &Path, space: &str) -> Option<Team> {
    load_teams(cfg).space.remove(space)
}

/// Generated once and kept, so rejoining from this machine is the same seat rather
/// than a second one.
fn device_of(cfg: &Path) -> String {
    let mut all = load_teams(cfg);
    if all.device.is_empty() {
        all.device = new_device();
        let _ = write_teams(cfg, &all);
    }
    all.device
}

/// The space's own file. Everything a team touches is inside `spaces/`.
fn space_file(cfg: &Path, space: &str) -> PathBuf {
    patchbay::space_path(cfg, Some(space))
}

/// Beside the space: the document both sides last agreed on. Kept because a merge
/// needs a base, and the server holds one revision with no history - once we edit
/// locally, the last agreed version exists nowhere else in the world.
fn base_path(cfg: &Path, space: &str) -> PathBuf {
    space_file(cfg, space).with_extension("toml.base")
}

/// Everything that means "we and the team now agree on this": the hash the next sync
/// compares against, the base a merge needs, and the file both live in. One function
/// because there are four places that agree and a fifth that forgets to is a machine
/// that quietly stops being able to merge.
fn settle(cfg: &Path, t: &mut Team, local: &str) -> Result<(), String> {
    t.synced = hash(local);
    // Best effort: without a base we fall back to today's pick-a-side conflict, which
    // is a worse answer but not a wrong one. Not a reason to fail a good sync.
    let _ = std::fs::write(base_path(cfg, &t.space), local);
    store(cfg, t)
}

fn store(cfg: &Path, t: &Team) -> Result<(), String> {
    let mut all = load_teams(cfg);
    all.space.insert(t.space.clone(), t.clone());
    write_teams(cfg, &all)
}

fn write_teams(cfg: &Path, all: &Teams) -> Result<(), String> {
    let path = team_path(cfg);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let body = toml::to_string(all).map_err(|e| e.to_string())?;
    write_private(&path, &body).map_err(|e| format!("{}: {e}", path.display()))
}

/// The code is the credential, so `team.toml` is the one file here nobody else on the
/// machine gets to read - unlike the config beside it, which is the whole point.
#[cfg(unix)]
fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` only applies to a file being created, so it misses one already sitting
    // there at the umask's 0644 - every install that joined a team before this line.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(body.as_bytes())
}

// ponytail: Windows inherits the profile directory's ACL, which is already owner-only.
// A real DACL is a windows-acl dependency for a case %APPDATA% covers.
#[cfg(not(unix))]
fn write_private(path: &Path, body: &str) -> std::io::Result<()> {
    std::fs::write(path, body)
}

/// Not a secret and not global - just something to tell two of your own machines
/// apart, so the seat count means what it says.
fn new_device() -> String {
    let seed = format!(
        "{}-{}-{:?}",
        std::process::id(),
        dirs::home_dir().unwrap_or_default().display(),
        std::time::SystemTime::now(),
    );
    hex(seed.as_bytes())[..24].to_string()
}

// ── the shared document ─────────────────────────────────────────────────────

/// A config we can't read is not an empty list. Handing back `""` here would look
/// exactly like the user deleting every jack, and the push branch would upload that
/// over the team's - one unreadable file on one machine wiping everyone's list.
fn read_local(cfg: &Path, space: &str) -> Result<String, String> {
    let path = space_file(cfg, space);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        // No file yet is the honest empty case: a space being made into a team.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// What we are willing to put. `replace_at` refuses a document that doesn't parse on
/// the way in, so pushing one leaves every teammate stuck on *our* syntax error until
/// we fix it - a broken file is this machine's problem and stays here.
fn to_push(local: &str) -> Result<String, String> {
    local
        .parse::<DocumentMut>()
        .map_err(|e| format!("that space doesn't parse, so it hasn't gone up: {e}"))?;
    Ok(local.to_string())
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(src: &str) -> String {
    hex(src.as_bytes())
}

/// A copy of what was here before the team's list replaced it. Every path that
/// overwrites a space file goes through `adopt`, so this is called from there.
fn backup(cfg: &Path, space: &str) {
    let path = space_file(cfg, space);
    if path.exists() {
        let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
    }
}

// ── the server ──────────────────────────────────────────────────────────────

/// The document and its `ETag`. Seats and the paid flag are advisory `x-` headers
/// our server adds; anywhere else simply doesn't send them.
struct Doc {
    doc: String,
    etag: String,
    seats: i64,
    paid: bool,
}

#[derive(Deserialize)]
struct Code {
    code: String,
}

struct Fail {
    /// A 409: someone else wrote between our read and our write.
    conflict: bool,
    /// A 402: the team is past its free seats, so writing is refused. Reading is
    /// not, which is the whole point of saying so separately.
    blocked: bool,
    msg: String,
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("no http client: {e}"))
}

fn send(req: reqwest::blocking::RequestBuilder) -> Result<Doc, Fail> {
    let res = req.send().map_err(|e| Fail {
        conflict: false,
        blocked: false,
        msg: format!("the team server didn't answer: {e}"),
    })?;
    let status = res.status();
    let head = |name: &str| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let (etag, seats, paid) = (
        head("etag"),
        head("x-seats").parse().unwrap_or(0),
        head("x-paid") == "1",
    );
    let body = res.text().unwrap_or_default();
    if !status.is_success() {
        return Err(refused(status, &body));
    }
    Ok(Doc { doc: body, etag, seats, paid })
}

/// The server's own error text is written for a person, so it is passed straight
/// through rather than wrapped in one of ours. A file store answers with something
/// else entirely, so a body that isn't our JSON is reported as the status it came
/// with rather than shown raw.
fn refused(status: reqwest::StatusCode, body: &str) -> Fail {
    Fail {
        // 412 is the standard answer to a stale If-Match; 409 is what this server said
        // before it spoke plain HTTP, and some stores still say it.
        conflict: matches!(status.as_u16(), 409 | 412),
        blocked: status.as_u16() == 402,
        msg: serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("the team server said {status}")),
    }
}

/// Only `POST /teams` still speaks JSON - it is the one call no file store has, and
/// the only thing it returns is a new code.
fn call<T: serde::de::DeserializeOwned>(req: reqwest::blocking::RequestBuilder) -> Result<T, Fail> {
    let res = req.send().map_err(|e| Fail {
        conflict: false,
        blocked: false,
        msg: format!("the team server didn't answer: {e}"),
    })?;
    let status = res.status();
    let body = res.text().unwrap_or_default();
    if !status.is_success() {
        return Err(refused(status, &body));
    }
    serde_json::from_str(&body).map_err(|e| Fail {
        conflict: false,
        blocked: false,
        msg: format!("the team server sent something unreadable: {e}"),
    })
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Our own server keeps the document at `/team`; anywhere else the URL already names
/// it. Told apart by whether the path is empty, which is what `https://host` gives.
fn doc_url(url: &str) -> String {
    let b = base(url);
    match b.split_once("://").map(|(_, rest)| rest.contains('/')) {
        Some(true) => b,
        _ => format!("{b}/team"),
    }
}

/// The code travels in a header, never the path - a logged URL is a leaked password.
/// With no code this is a plain `GET` of whatever the URL points at.
fn signed(cfg: &Path, t: &Team, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
    match &t.code {
        Some(code) => req.header("x-team", code).header("x-device", device_of(cfg)),
        None => req,
    }
}

fn fetch(cfg: &Path, t: &Team) -> Result<Doc, Fail> {
    let c = client().map_err(|msg| Fail { conflict: false, blocked: false, msg })?;
    send(signed(cfg, t, c.get(doc_url(&t.url))))
}

fn put(cfg: &Path, t: &Team, doc: &str, etag: &str) -> Result<Doc, Fail> {
    let c = client().map_err(|msg| Fail { conflict: false, blocked: false, msg })?;
    send(
        signed(cfg, t, c.put(doc_url(&t.url)))
            .header("if-match", etag)
            .header("content-type", "text/plain; charset=utf-8")
            .body(doc.to_string()),
    )
}

// ── what the window sees ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Status {
    /// Which space this is the state of.
    pub space: String,
    /// synced · conflict · blocked · offline · error, where `offline` is the server's
    /// fault and `error` is this machine's - a file that won't read or parse. Telling
    /// them apart matters: one clears itself, the other needs you.
    pub state: &'static str,
    pub url: String,
    pub code: String,
    pub seats: i64,
    pub paid: bool,
    pub error: Option<String>,
    /// The config file on this disk was rewritten, so whatever read it is stale.
    pub changed: bool,
}

fn ok(t: &Team, d: &Doc, state: &'static str) -> Status {
    Status {
        space: t.space.clone(),
        state,
        url: t.url.clone(),
        code: t.code.clone().unwrap_or_default(),
        seats: d.seats,
        paid: d.paid,
        error: None,
        changed: false,
    }
}

fn stuck(t: &Team, state: &'static str, msg: String) -> Status {
    Status {
        space: t.space.clone(),
        state,
        url: t.url.clone(),
        code: t.code.clone().unwrap_or_default(),
        seats: 0,
        paid: false,
        error: Some(msg),
        changed: false,
    }
}

pub fn sync() -> Vec<Status> {
    sync_at(&patchbay::config_path())
}

/// Every team space, in turn. No teams is an empty list, and no request at all.
pub fn sync_at(cfg: &Path) -> Vec<Status> {
    load_teams(cfg)
        .space
        .into_keys()
        .map(|space| sync_space(cfg, &space))
        .collect()
}

/// Fetch, then whichever of push or adopt applies. Idempotent, so every caller - the
/// window regaining focus, the end of an edit, the settings sheet - is this one call.
fn sync_space(cfg: &Path, space: &str) -> Status {
    // Held before the load, so a second sync reads the state the first one stored
    // rather than the state it started from.
    let _one_at_a_time = RUNNING.lock().unwrap_or_else(|e| e.into_inner());

    let mut t = match load(cfg, space) {
        Some(t) => t,
        None => return stuck(&Team { space: space.to_string(), ..Team::default() },
                             "error", format!("no team on \"{space}\" any more")),
    };
    let local = match read_local(cfg, space) {
        Ok(s) => s,
        Err(e) => return stuck(&t, "error", e),
    };
    let remote = match fetch(cfg, &t) {
        Ok(d) => d,
        Err(f) => return stuck(&t, "offline", f.msg),
    };

    let we_moved = hash(&local) != t.synced;
    let they_moved = remote.etag != t.etag;

    // A space with no code is a subscription: it takes what arrives and pushes
    // nothing. Editing it locally is the one thing that needs saying out loud.
    if t.code.is_none() && we_moved {
        return stuck(&t, "readonly", "this space is read-only, so your edits stay here".into());
    }

    match (we_moved, they_moved) {
        // Both sides moved, and only a person knows which one is right.
        // Both sides moved. If they moved *different* devices there is nothing for a
        // person to decide, so merge and push. Only the same key on both sides is a
        // conflict anyone has to answer.
        (true, true) => match std::fs::read_to_string(base_path(cfg, space))
            .ok()
            .and_then(|base| merge(&base, &local, &remote.doc))
        {
            None => stuck(&t, "conflict", "your list and the team's have both changed".into()),
            Some(merged) => match put(cfg, &t, &merged, &remote.etag) {
                Ok(d) => {
                    match config::replace_at(&space_file(cfg, space), &merged)
                        .and_then(|()| read_local(cfg, space))
                        .and_then(|landed| {
                            t.etag = d.etag.clone();
                            settle(cfg, &mut t, &landed)
                        }) {
                        Ok(()) => Status { changed: true, ..ok(&t, &d, "synced") },
                        Err(e) => stuck(&t, "error", e),
                    }
                }
                Err(f) if f.blocked => stuck(&t, "blocked", f.msg),
                // Someone wrote again while we were merging; the next sync re-reads
                // and merges against whatever is there now.
                Err(f) => stuck(&t, "conflict", f.msg),
            },
        },
        (true, false) => match to_push(&local) {
            Err(e) => stuck(&t, "error", e),
            Ok(doc) => match put(cfg, &t, &doc, &t.etag) {
                Ok(d) => {
                    t.etag = d.etag.clone();
                    match settle(cfg, &mut t, &local) {
                        Ok(()) => ok(&t, &d, "synced"),
                        Err(e) => stuck(&t, "error", e),
                    }
                }
                // Someone wrote between our fetch and our put - the next sync sees it
                // as the conflict it is, but say so now rather than reporting success.
                Err(f) if f.conflict => stuck(&t, "conflict", f.msg),
                Err(f) if f.blocked => stuck(&t, "blocked", f.msg),
                Err(f) => stuck(&t, "offline", f.msg),
            },
        },
        (false, true) => match adopt(cfg, &mut t, &remote, &local) {
            Ok(()) => Status { changed: true, ..ok(&t, &remote, "synced") },
            Err(e) => stuck(&t, "error", e),
        },
        (false, false) => ok(&t, &remote, "synced"),
    }
}

/// Every leaf that differs, walking as deep as both sides stay tables. Depth is the
/// whole point: `[jack.web]` and `[jack.db]` are both edits to `jack`, and stopping at
/// the top would call two people adding two devices the same change - stopping one
/// level down would say the same about two people editing two fields of one device.
/// Compared by value, not by text: a reformat is not an edit.
type Paths = std::collections::BTreeSet<Vec<String>>;

fn changed_paths(a: &str, b: &str) -> Option<Paths> {
    let (ta, tb) = (a.parse::<toml::Table>().ok()?, b.parse::<toml::Table>().ok()?);
    let mut out = Paths::new();
    walk(&ta, &tb, &mut Vec::new(), &mut out);
    Some(out)
}

fn walk(a: &toml::Table, b: &toml::Table, at: &mut Vec<String>, out: &mut Paths) {
    for k in a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>() {
        match (a.get(k), b.get(k)) {
            (Some(x), Some(y)) if x == y => {}
            (Some(toml::Value::Table(x)), Some(toml::Value::Table(y))) => {
                at.push(k.clone());
                walk(x, y, at, out);
                at.pop();
            }
            (None, None) => {}
            // A leaf, or a key that is a table on one side and something else on the
            // other - which is a disagreement about the shape, not a mergeable edit.
            _ => {
                at.push(k.clone());
                out.insert(at.clone());
                at.pop();
            }
        }
    }
}

/// Copy one path from `from` into `doc`, or delete it where `from` hasn't got it.
/// Walks the same depth `changed_paths` produced, making tables on the way down.
fn apply(doc: &mut DocumentMut, from: &DocumentMut, path: &[String]) {
    let Some((last, parents)) = path.split_last() else { return };

    let mut src = from.as_table().get(&parents.first().cloned().unwrap_or_default());
    for p in parents.iter().skip(1) {
        src = src.and_then(|i| i.as_table()).and_then(|t| t.get(p));
    }
    let incoming = match parents.is_empty() {
        true => from.as_table().get(last).cloned(),
        false => src.and_then(|i| i.as_table()).and_then(|t| t.get(last)).cloned(),
    };

    // Down to the table that holds the leaf, creating implicit ones so a new
    // `[jack.db]` never emits a bare `[jack]` header of its own.
    let mut table = doc.as_table_mut();
    for p in parents {
        let entry = table.entry(p).or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        });
        match entry.as_table_mut() {
            Some(t) => table = t,
            None => return,
        }
    }

    match incoming {
        Some(item) => {
            table.insert(last, item);
        }
        None => {
            let orphan = config::orphan_comments(table, last);
            config::rehome_comments(doc, orphan);
        }
    }
}

/// The one merge worth doing, and it is not a text merge. The document is a map of
/// named devices, so when the entries each side touched don't overlap there is nothing
/// to reconcile: Alice adding `jack.db` and Bob adding `jack.cache` is two edits to one
/// file, not a disagreement. The same entry on both sides is a real conflict and still
/// goes to the user.
///
/// What is left as a conflict is the same *leaf* on both sides - two people setting
/// `jack.web.host` to two different addresses, which is the one case where a machine
/// picking for you would be picking wrong half the time.
fn merge(base: &str, ours: &str, theirs: &str) -> Option<String> {
    let mine = changed_paths(base, ours)?;
    let yours = changed_paths(base, theirs)?;
    if !mine.is_disjoint(&yours) {
        return None;
    }
    let mut doc = base.parse::<DocumentMut>().ok()?;
    let (o, t) = (ours.parse::<DocumentMut>().ok()?, theirs.parse::<DocumentMut>().ok()?);
    for (paths, from) in [(&mine, &o), (&yours, &t)] {
        for path in paths {
            apply(&mut doc, from, path);
        }
    }
    Some(doc.to_string())
}

/// Take the team's document as this space's.
fn adopt(cfg: &Path, t: &mut Team, remote: &Doc, local: &str) -> Result<(), String> {
    // Even an ordinary pull is worth a copy: the document it replaces exists nowhere
    // else - the server keeps one revision and no history, and every other device is
    // adopting this same replacement. One teammate truncating their space would
    // otherwise take the list off every machine with nothing left to put back.
    // An empty space is the join case, and has nothing to lose.
    if !local.trim().is_empty() && hash(local) != hash(&remote.doc) {
        backup(cfg, &t.space);
    }
    config::replace_at(&space_file(cfg, &t.space), &remote.doc)?;
    t.etag = remote.etag.clone();
    // Hashed from the file as it now stands, not from what arrived: writing a parsed
    // document back moves whitespace, and a hash of the wrong one reads as a local
    // edit and pushes the team's own list straight back at them.
    let landed = read_local(cfg, &t.space)?;
    settle(cfg, t, &landed)
}

pub fn join(name: &str, url: &str, code: &str) -> Result<Status, String> {
    join_at(&patchbay::config_path(), name, url, code)
}

/// Joining makes a *new* space and puts the team's list in it. Nothing you already
/// have is read, replaced or uploaded - that is the whole reason a team is a space
/// of its own rather than your config file.
pub fn join_at(cfg: &Path, name: &str, url: &str, code: &str) -> Result<Status, String> {
    if !crate::is_web_url(url) {
        return Err("a team server address starts with http:// or https://".into());
    }
    let code = code.trim();
    let space = config::create_space_at(cfg, name)?;
    let mut t = Team {
        url: base(url),
        // No code at all is a plain URL: whatever is there, read-only.
        code: (!code.is_empty()).then(|| code.to_string()),
        etag: String::new(),
        synced: String::new(),
        space,
    };
    let remote = match fetch(cfg, &t) {
        Ok(d) => d,
        // The empty space we just made would otherwise sit there looking like a team.
        Err(f) => {
            let _ = config::delete_space_at(cfg, &t.space);
            return Err(f.msg);
        }
    };
    let local = read_local(cfg, &t.space)?;
    adopt(cfg, &mut t, &remote, &local)?;
    Ok(Status { changed: true, ..ok(&t, &remote, "synced") })
}

pub fn create(space: &str, url: &str) -> Result<Status, String> {
    create_at(&patchbay::config_path(), space, url)
}

/// Hand an existing space to a new team: the devices already in it become the team's
/// list. Making a space and filling it first is how you decide what gets shared.
pub fn create_at(cfg: &Path, space: &str, url: &str) -> Result<Status, String> {
    if !crate::is_web_url(url) {
        return Err("a team server address starts with http:// or https://".into());
    }
    if load(cfg, space).is_some() {
        return Err(format!("\"{space}\" is already a team"));
    }
    let local = read_local(cfg, space)?;
    let doc = to_push(&local)?;

    let made: Code = call(client()?.post(format!("{}/teams", base(url))))
        .map_err(|f| f.msg)?;
    let mut t = Team {
        url: base(url),
        code: Some(made.code),
        etag: String::new(),
        synced: String::new(),
        space: space.to_string(),
    };
    // A new team is empty but its ETag is not nothing, and only the server knows
    // what it starts at - so ask rather than assume, once, on the one call where an
    // extra round trip costs nothing.
    let empty = fetch(cfg, &t).map_err(|f| f.msg)?;
    let d = put(cfg, &t, &doc, &empty.etag).map_err(|f| f.msg)?;
    t.etag = d.etag.clone();
    settle(cfg, &mut t, &local)?;
    Ok(ok(&t, &d, "synced"))
}

pub fn resolve(space: &str, keep: &str) -> Result<Status, String> {
    resolve_at(&patchbay::config_path(), space, keep)
}

/// `mine` overwrites the team's copy, `theirs` overwrites ours - the two ways out of
/// a conflict, both of them somebody's deliberate choice, and both keeping a `.bak`.
pub fn resolve_at(cfg: &Path, space: &str, keep: &str) -> Result<Status, String> {
    let mut t = load(cfg, space).ok_or_else(|| format!("\"{space}\" isn't a team"))?;
    let remote = fetch(cfg, &t).map_err(|f| f.msg)?;
    let local = read_local(cfg, space)?;

    if keep == "mine" {
        if t.code.is_none() {
            return Err(format!("\"{space}\" is read-only, so there is nothing to push"));
        }
        let d = put(cfg, &t, &to_push(&local)?, &remote.etag).map_err(|f| f.msg)?;
        t.etag = d.etag.clone();
        settle(cfg, &mut t, &local)?;
        return Ok(ok(&t, &d, "synced"));
    }
    adopt(cfg, &mut t, &remote, &local)?;
    Ok(Status { changed: true, ..ok(&t, &remote, "synced") })
}

pub fn leave(space: &str) -> Result<(), String> {
    leave_at(&patchbay::config_path(), space)
}

/// Leaving keeps the space - it is a config file on this disk, and it stops being
/// anyone else's business the moment nothing is syncing it.
pub fn leave_at(cfg: &Path, space: &str) -> Result<(), String> {
    let mut all = load_teams(cfg);
    all.space.remove(space);
    let _ = std::fs::remove_file(base_path(cfg, space));
    write_teams(cfg, &all)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    /// Your own list. It is the main config, it is not a team space, and no test here
    /// may see it move.
    const MINE: &str = "# my own\n[jack.laptop]\nhost = \"192.168.1.9\"\n";
    /// The space that gets shared.
    const TEAM: &str = "# the team's\n[jack.web]\nhost = \"10.0.0.4\"\n";

    /// Stands in for `server/`: one document, one version, and a write that only
    /// lands if the version still matches. Close enough to pin our half of the
    /// contract - the headers, the conflict, and who overwrites whom - without a
    /// second process or a second crate.
    fn stub(doc: &str) -> (String, Arc<Mutex<(String, i64)>>) {
        let state = Arc::new(Mutex::new((doc.to_string(), 1)));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let shared_state = state.clone();

        std::thread::spawn(move || {
            for mut sock in listener.incoming().flatten() {
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let (mut head, mut len) = (String::new(), 0usize);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                let body = String::from_utf8_lossy(&body).to_string();
                let lower = head.to_ascii_lowercase();

                let mut st = shared_state.lock().unwrap();
                // Whatever the client sent as If-Match, as a number.
                let sent = lower
                    .lines()
                    .find_map(|l| l.strip_prefix("if-match:"))
                    .map(|v| v.trim().trim_start_matches("w/").trim_matches('"'))
                    .and_then(|v| v.parse::<i64>().ok());

                let (code, payload) = if head.starts_with("POST") {
                    // Making a team is the one call with no code yet - it hands one back.
                    (200, "{\"code\":\"abcd-efgh\"}".to_string())
                } else if !lower.contains("x-team:") || !lower.contains("x-device:") {
                    (400, "{\"error\":\"missing header\"}".to_string())
                } else if head.starts_with("GET") {
                    (200, st.0.clone())
                } else if sent != Some(st.1) {
                    (412, "{\"error\":\"the config changed underneath you\"}".to_string())
                } else {
                    st.0 = body.clone();
                    st.1 += 1;
                    (200, st.0.clone())
                };
                let version = st.1;
                drop(st);
                let _ = write!(
                    sock,
                    "HTTP/1.1 {code} x\r\ncontent-type: text/plain\r\netag: \"{version}\"\r\nx-seats: 2\r\nx-paid: 0\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        (url, state)
    }

    /// A plain document over HTTP: no code, no writes. What a raw git URL or a bucket
    /// object looks like, and the reason the transport is ETag rather than our own JSON.
    fn static_stub(doc: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/list.toml", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut sock in listener.incoming().flatten() {
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let (code, payload) = match head.starts_with("GET") {
                    true => (200, doc),
                    false => (405, ""),
                };
                let _ = write!(
                    sock,
                    "HTTP/1.1 {code} x\r\ncontent-type: text/plain\r\netag: \"v1\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        url
    }

    /// A scratch config directory with your own list in it, and nothing shared yet.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("patchbay-team-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("patchbay.toml");
        std::fs::write(&cfg, MINE).unwrap();
        cfg
    }
    /// Where the one space these tests use lives.
    fn sp(cfg: &Path) -> PathBuf {
        space_file(cfg, "acme")
    }
    fn fill(cfg: &Path, body: &str) {
        std::fs::create_dir_all(sp(cfg).parent().unwrap()).unwrap();
        std::fs::write(sp(cfg), body).unwrap();
    }
    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }
    /// These tests run one team space, so a sync has exactly one answer.
    fn one(cfg: &Path) -> Status {
        let mut all = sync_at(cfg);
        assert_eq!(all.len(), 1, "expected exactly one team space");
        all.pop().unwrap()
    }

    #[test]
    fn making_a_team_from_a_space_sends_that_space_and_a_later_edit_pushes() {
        let (url, server) = stub("");
        let cfg = scratch("create");
        fill(&cfg, TEAM);

        let s = create_at(&cfg, "acme", &url).unwrap();
        assert_eq!(s.state, "synced", "{:?}", s.error);
        assert!(server.lock().unwrap().0.contains("[jack.web]"), "the space didn't go up");
        assert!(!server.lock().unwrap().0.contains("[jack.laptop]"), "your own list went up");

        // Nothing has changed on either side, so this neither pushes nor writes.
        let before = server.lock().unwrap().1;
        assert_eq!(one(&cfg).state, "synced");
        assert_eq!(server.lock().unwrap().1, before, "an idle sync wrote anyway");

        fill(&cfg, &format!("{TEAM}\n[jack.db]\nhost = \"10.0.0.9\"\n"));
        assert_eq!(one(&cfg).state, "synced");
        assert!(server.lock().unwrap().0.contains("[jack.db]"), "the edit stayed here");
    }

    /// The reason a team is a space of its own: joining one can't touch what you had.
    #[test]
    fn joining_makes_a_new_space_and_leaves_your_own_list_alone() {
        let (url, _server) = stub("[jack.theirs]\nhost = \"10.0.0.2\"\n");
        let cfg = scratch("join");

        let s = join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();
        assert_eq!(s.state, "synced", "{:?}", s.error);
        assert_eq!(s.space, "acme");
        assert!(read(&sp(&cfg)).contains("[jack.theirs]"), "{}", read(&sp(&cfg)));
        assert_eq!(read(&cfg), MINE, "joining a team rewrote your own list");
        assert!(!sp(&cfg).with_extension("toml.bak").exists(), "a fresh space had nothing to back up");
    }

    #[test]
    fn a_teammates_change_lands_here_and_leaves_your_own_list_alone() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("pull");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        {
            let mut st = server.lock().unwrap();
            st.0 = "[jack.db]\nhost = \"10.0.0.9\"\n".into();
            st.1 += 1;
        }
        let s = one(&cfg);
        assert_eq!(s.state, "synced", "{:?}", s.error);
        assert!(s.changed, "the window wasn't told the file moved");
        assert!(read(&sp(&cfg)).contains("[jack.db]"), "{}", read(&sp(&cfg)));
        assert!(!read(&sp(&cfg)).contains("[jack.web]"), "the team's list is the list");
        assert_eq!(read(&cfg), MINE, "a pull reached outside its space");

        // And once adopted it is the agreed document, so nothing bounces back.
        let version = server.lock().unwrap().1;
        assert_eq!(one(&cfg).state, "synced");
        assert_eq!(server.lock().unwrap().1, version, "the pull pushed itself back");
    }

    /// Two people adding two different devices is not a disagreement, and making
    /// someone pick a side - throwing the loser's work to a .bak - is the thing that
    /// makes a shared list not worth sharing.
    #[test]
    fn edits_to_different_devices_merge_instead_of_colliding() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("merge");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        // We add one device; a colleague adds another, to the list we agreed on.
        fill(&cfg, &format!("{TEAM}\n[jack.mine]\nhost = \"10.0.0.1\"\n"));
        {
            let mut st = server.lock().unwrap();
            st.0 = format!("{TEAM}\n[jack.theirs]\nhost = \"10.0.0.2\"\n");
            st.1 += 1;
        }

        let s = one(&cfg);
        assert_eq!(s.state, "synced", "{:?}", s.error);
        let here = read(&sp(&cfg));
        assert!(here.contains("[jack.mine]"), "our device was dropped: {here}");
        assert!(here.contains("[jack.theirs]"), "their device never arrived: {here}");
        assert!(here.contains("[jack.web]"), "the list we started from went missing: {here}");

        let up = server.lock().unwrap().0.clone();
        assert!(up.contains("[jack.mine]") && up.contains("[jack.theirs]"), "{up}");
        // And the merge is the new agreement, so nothing bounces back.
        let version = server.lock().unwrap().1;
        assert_eq!(one(&cfg).state, "synced");
        assert_eq!(server.lock().unwrap().1, version, "the merge pushed itself again");
    }

    /// merge walks to the leaf, so only the same field on both sides collides.
    #[test]
    fn two_fields_of_one_device_merge_and_the_same_field_does_not() {
        let base = "[jack.web]\nhost = \"10.0.0.4\"\nuser = \"deploy\"\n";
        let ours = "[jack.web]\nhost = \"10.0.0.4\"\nuser = \"root\"\n";
        let theirs = "[jack.web]\nhost = \"10.0.0.4\"\nuser = \"deploy\"\nport = 2222\n";

        let merged = merge(base, ours, theirs).expect("different fields should merge");
        assert!(merged.contains("root"), "our field was dropped: {merged}");
        assert!(merged.contains("2222"), "their field never arrived: {merged}");

        // The same field, two answers: nobody but a person can pick.
        let clash = "[jack.web]\nhost = \"10.0.0.4\"\nuser = \"admin\"\n";
        assert!(merge(base, ours, clash).is_none(), "a real collision was merged away");
    }

    /// A device deleted here and untouched there stays deleted - a merge that quietly
    /// resurrected what someone removed would be worse than refusing to merge at all.
    /// resurrected what someone removed would be worse than refusing to merge at all.
    #[test]
    fn a_deletion_survives_the_merge() {
        let base = "[jack.web]\nhost = \"10.0.0.4\"\n\n[jack.db]\nhost = \"10.0.0.5\"\n";
        let ours = "[jack.db]\nhost = \"10.0.0.5\"\n";
        let theirs = "[jack.web]\nhost = \"10.0.0.4\"\n\n[jack.db]\nhost = \"10.0.0.5\"\n\n[jack.new]\nhost = \"10.0.0.6\"\n";

        let merged = merge(base, ours, theirs).expect("a delete and an add don't overlap");
        assert!(!merged.contains("jack.web"), "the deleted device came back: {merged}");
        assert!(merged.contains("jack.new"), "their addition was lost: {merged}");
        assert!(merged.contains("jack.db"), "an untouched device went missing: {merged}");
    }

    /// The same device on both sides is a real disagreement, and stays one.
    /// The same device on both sides is a real disagreement, and stays one.
    #[test]
    fn the_same_device_on_both_sides_is_still_a_conflict() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("collide");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        fill(&cfg, &TEAM.replace("10.0.0.4", "10.0.0.44"));
        {
            let mut st = server.lock().unwrap();
            st.0 = TEAM.replace("10.0.0.4", "10.0.0.99");
            st.1 += 1;
        }
        let s = one(&cfg);
        assert_eq!(s.state, "conflict", "{:?}", s.error);
        assert!(read(&sp(&cfg)).contains("10.0.0.44"), "a conflict overwrote the local file");
    }

    #[test]
    fn both_sides_moving_is_a_conflict_until_someone_picks() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("conflict");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        {
            let mut st = server.lock().unwrap();
            st.0 = "[jack.web]\nhost = \"10.9.9.9\"\n[jack.theirs]\nhost = \"10.0.0.2\"\n".into();
            st.1 += 1;
        }
        // Both moved `jack.web` - ours by rewriting its host, theirs by rewriting it
        // differently, while we also added a device. Overlapping, so nobody can merge.
        fill(&cfg, &format!("{}\n[jack.mine]\nhost = \"10.0.0.1\"\n", TEAM.replace("10.0.0.4", "10.0.0.44")));
        let s = one(&cfg);
        assert_eq!(s.state, "conflict", "{:?}", s.error);
        assert!(read(&sp(&cfg)).contains("[jack.mine]"), "a conflict overwrote the local file");

        // Taking theirs keeps ours beside it rather than dropping it.
        let s = resolve_at(&cfg, "acme", "theirs").unwrap();
        assert_eq!(s.state, "synced");
        assert!(read(&sp(&cfg)).contains("[jack.theirs]"), "{}", read(&sp(&cfg)));
        assert!(
            read(&sp(&cfg).with_extension("toml.bak")).contains("[jack.mine]"),
            "the list we dropped wasn't kept"
        );
        assert_eq!(one(&cfg).state, "synced", "still in conflict after resolving");
    }

    #[test]
    fn pushing_mine_wins_the_conflict_the_other_way() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("mine");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        fill(&cfg, &format!("{}\n[jack.mine]\nhost = \"10.0.0.1\"\n", TEAM.replace("10.0.0.4", "10.0.0.44")));
        {
            let mut st = server.lock().unwrap();
            st.0 = TEAM.replace("10.0.0.4", "10.0.0.99");
            st.1 += 1;
        }
        assert_eq!(one(&cfg).state, "conflict");

        assert_eq!(resolve_at(&cfg, "acme", "mine").unwrap().state, "synced");
        assert!(server.lock().unwrap().0.contains("[jack.mine]"));
        assert_eq!(one(&cfg).state, "synced");
    }

    #[test]
    fn no_team_means_no_request_at_all_and_a_dead_server_is_not_a_lost_list() {
        let cfg = scratch("off");
        assert!(sync_at(&cfg).is_empty(), "a sync with no team spaces asked someone something");
        assert!(leave_at(&cfg, "acme").is_ok(), "leaving when not in a team is not an error");

        // A port nothing listens on: the state says so and the space is untouched.
        fill(&cfg, TEAM);
        store(
            &cfg,
            &Team {
                url: "http://127.0.0.1:1".into(),
                code: Some("abcd".into()),
                etag: "\"1\"".into(),
                synced: String::new(),
                space: "acme".into(),
            },
        )
        .unwrap();
        let s = one(&cfg);
        assert_eq!(s.state, "offline");
        assert!(s.error.is_some());
        assert_eq!(read(&sp(&cfg)), TEAM);

        // Leaving keeps the space; it is a config file, and it is already on this disk.
        leave_at(&cfg, "acme").unwrap();
        assert!(sync_at(&cfg).is_empty());
        assert_eq!(read(&sp(&cfg)), TEAM, "leaving took the list with it");
    }

    #[test]
    fn a_space_we_cannot_read_is_not_an_empty_list() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("unreadable");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        // A path that exists but doesn't read as a file. Turning every read error into
        // "" would be indistinguishable from deleting every device.
        std::fs::remove_file(sp(&cfg)).unwrap();
        std::fs::create_dir(sp(&cfg)).unwrap();
        let s = one(&cfg);
        assert_eq!(s.state, "error", "{:?}", s.error);
        assert!(server.lock().unwrap().0.contains("[jack.web]"), "the team's list was wiped");
    }

    #[test]
    fn a_space_that_does_not_parse_stays_on_this_machine() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("broken");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        fill(&cfg, "[jack.web\nhost = ");
        let s = one(&cfg);
        assert_eq!(s.state, "error", "{:?}", s.error);
        assert!(
            server.lock().unwrap().0.contains("[jack.web]"),
            "a file that doesn't parse went up, and every teammate chokes on it"
        );
    }

    #[test]
    fn an_ordinary_pull_keeps_what_it_replaced() {
        let (url, server) = stub(TEAM);
        let cfg = scratch("pullbak");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();

        // A teammate whose list lost everything but one host. We haven't touched
        // ours, so this is the quiet path - and the one with the most to lose.
        {
            let mut st = server.lock().unwrap();
            st.0 = "[jack.only]\nhost = \"10.0.0.9\"\n".into();
            st.1 += 1;
        }
        assert_eq!(one(&cfg).state, "synced");
        assert!(
            read(&sp(&cfg).with_extension("toml.bak")).contains("[jack.web]"),
            "the list the pull replaced wasn't kept anywhere"
        );
    }

    #[test]
    fn two_syncs_at_once_do_not_invent_a_conflict() {
        let (url, _server) = stub(TEAM);
        let cfg = scratch("race");
        join_at(&cfg, "acme", &url, "abcd-efgh").unwrap();
        fill(&cfg, &format!("{TEAM}\n[jack.db]\nhost = \"10.0.0.9\"\n"));

        // The window regaining focus while an edit finishes. Both must land on the
        // same answer; one racing the other into a 409 is not a conflict.
        let (a, b) = (cfg.clone(), cfg.clone());
        let one_ = std::thread::spawn(move || sync_at(&a).pop().unwrap().state);
        let two = std::thread::spawn(move || sync_at(&b).pop().unwrap().state);
        assert_eq!(one_.join().unwrap(), "synced");
        assert_eq!(two.join().unwrap(), "synced");
    }

    #[cfg(unix)]
    #[test]
    fn the_team_code_is_not_readable_by_anyone_else_on_the_machine() {
        use std::os::unix::fs::PermissionsExt;
        let cfg = scratch("perms");
        let t = Team {
            url: "http://127.0.0.1:1".into(),
            code: Some("abcd-efgh".into()),
            etag: "\"1\"".into(),
            synced: String::new(),
            space: "acme".into(),
        };
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        store(&cfg, &t).unwrap();
        assert_eq!(mode(&team_path(&cfg)), 0o600, "the code was world-readable");

        // A file from before this was tightened is fixed on the next write, not left.
        std::fs::set_permissions(team_path(&cfg), std::fs::Permissions::from_mode(0o644)).unwrap();
        store(&cfg, &t).unwrap();
        assert_eq!(mode(&team_path(&cfg)), 0o600, "an existing file kept its old mode");

        assert_eq!(read(&cfg), MINE);
    }

    #[test]
    fn only_an_http_address_is_a_team_server() {
        let cfg = scratch("url");
        for bad in ["file:///etc/passwd", "ssh://box", "127.0.0.1:8787", ""] {
            assert!(join_at(&cfg, "acme", bad, "abcd").is_err(), "{bad:?} should be refused");
            assert!(create_at(&cfg, "acme", bad).is_err(), "{bad:?} should be refused");
        }
    }

    /// Our own server keeps the document at `/team`; anywhere else the URL already
    /// names the thing to fetch, and appending to it would ask for the wrong file.
    #[test]
    fn a_bare_host_gets_our_path_and_a_real_url_is_left_alone() {
        assert_eq!(doc_url("https://patchbay.example"), "https://patchbay.example/team");
        assert_eq!(doc_url("https://patchbay.example/"), "https://patchbay.example/team");
        assert_eq!(
            doc_url("https://bucket.example/hosts/team.toml"),
            "https://bucket.example/hosts/team.toml"
        );
    }

    /// A space with no code is a subscription to whatever is at that URL: it takes
    /// what arrives and never tries to write.
    #[test]
    fn a_space_with_no_code_reads_a_plain_url_and_never_writes() {
        let url = static_stub("[jack.shared]\nhost = \"10.0.0.7\"\n");
        let cfg = scratch("readonly");

        let s = join_at(&cfg, "acme", &url, "").unwrap();
        assert_eq!(s.state, "synced", "{:?}", s.error);
        assert!(read(&sp(&cfg)).contains("[jack.shared]"), "{}", read(&sp(&cfg)));
        assert!(s.code.is_empty(), "a read-only space has no code to show");

        // Nothing changed either side, and nothing was written - the stub 405s a put.
        assert_eq!(one(&cfg).state, "synced");

        // Editing it locally is the one thing that needs saying out loud, rather than
        // silently failing on every sync or silently throwing the edit away.
        fill(&cfg, "[jack.mine]\nhost = \"10.0.0.1\"\n");
        let s = one(&cfg);
        assert_eq!(s.state, "readonly", "{:?}", s.error);
        assert!(read(&sp(&cfg)).contains("[jack.mine]"), "the edit was thrown away");
        assert!(resolve_at(&cfg, "acme", "mine").is_err(), "pushed to a read-only space");
    }

    /// A join that can't reach the server must not leave an empty space behind
    /// looking like a team you are in.
    #[test]
    fn a_join_that_fails_leaves_nothing_behind() {
        let cfg = scratch("failed");
        assert!(join_at(&cfg, "acme", "http://127.0.0.1:1", "abcd").is_err());
        assert!(!sp(&cfg).exists(), "an empty space was left lying around");
        assert!(sync_at(&cfg).is_empty());
    }
}
