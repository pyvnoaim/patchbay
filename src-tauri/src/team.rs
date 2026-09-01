//! Team sync: this machine's config file, mirrored through the team server.
//!
//! The whole config document is the shared thing. The server stores a string and
//! never parses it, so there is no schema on that side and nothing to merge here —
//! the two ends only ever agree or disagree.
//!
//! Our half lives in `team.toml` *beside* the config, never in it, because the config
//! is what gets uploaded: a team code in there would be a credential in a file the
//! whole team reads, and the device id would stop counting seats the moment it was
//! shared. `[settings]` is stripped on the way out and kept on the way in for the same
//! reason — they are this machine's preferences, not the team's device list.

use crate::{config, patchbay};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use toml_edit::DocumentMut;

/// Long enough for a laptop on hotel wifi, short enough that a dead server doesn't
/// hold the window's focus handler.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Team {
    pub url: String,
    pub code: String,
    /// Opaque, per install. The server counts seats by it.
    pub device: String,
    /// The server version and shared-document hash we last agreed on. Between them
    /// they say who moved: a different hash means we edited, a different version
    /// means they did, and both at once is the one case nobody can merge for us.
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub synced: String,
}

/// Beside the config, and derived from it, so `$PATCHBAY_CONFIG` moves both together
/// and the tests get a scratch team the same way they get a scratch config.
fn team_path(cfg: &Path) -> PathBuf {
    cfg.with_file_name("team.toml")
}

fn load(cfg: &Path) -> Option<Team> {
    let t: Team = toml::from_str(&std::fs::read_to_string(team_path(cfg)).ok()?).ok()?;
    (!t.code.is_empty() && !t.url.is_empty()).then_some(t)
}

fn store(cfg: &Path, t: &Team) -> Result<(), String> {
    let path = team_path(cfg);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let body = toml::to_string(t).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))
}

/// Not a secret and not global — just something to tell two of your own machines
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

fn read_local(cfg: &Path) -> String {
    std::fs::read_to_string(cfg).unwrap_or_default()
}

fn table_of(src: &str, key: &str) -> Option<toml_edit::Item> {
    src.parse::<DocumentMut>().ok()?.as_table().get(key).cloned()
}

/// What actually travels: everything but `[settings]`. Sharing those would mean a
/// colleague ticking "connect in the system terminal" ticks it on your laptop too.
fn shared(src: &str) -> String {
    match src.parse::<DocumentMut>() {
        Ok(mut doc) => {
            // Straight `remove` would take the lines above `[settings]` with it, and
            // a file header is nobody's preference — same rehoming as a deleted table.
            let orphan = config::orphan_comments(doc.as_table_mut(), "settings");
            config::rehome_comments(&mut doc, orphan);
            doc.to_string()
        }
        // Unparseable is the local file's problem, not something to silently drop
        // half of — send it as it stands and let the round trip stay honest.
        Err(_) => src.to_string(),
    }
}

/// The team's document with our own `[settings]` put back, ready to be written here.
fn with_local_settings(theirs: &str, ours: &str) -> String {
    let Some(settings) = table_of(ours, "settings") else {
        return theirs.to_string();
    };
    match theirs.parse::<DocumentMut>() {
        Ok(mut doc) => {
            doc.as_table_mut().insert("settings", settings);
            doc.to_string()
        }
        Err(_) => theirs.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(src: &str) -> String {
    hex(shared(src).as_bytes())
}

/// A copy of what was here before the team's list replaced it. Only on the two paths
/// that deliberately overwrite local work — an ordinary pull has nothing to lose.
fn backup(cfg: &Path) {
    if cfg.exists() {
        let _ = std::fs::copy(cfg, cfg.with_extension("toml.bak"));
    }
}

// ── the server ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Doc {
    doc: String,
    version: i64,
    #[serde(default)]
    seats: i64,
    #[serde(default)]
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

/// The server's own error text is written for a person, so it is passed straight
/// through rather than wrapped in one of ours.
fn call<T: serde::de::DeserializeOwned>(req: reqwest::blocking::RequestBuilder) -> Result<T, Fail> {
    let res = req.send().map_err(|e| Fail {
        conflict: false,
        blocked: false,
        msg: format!("the team server didn't answer: {e}"),
    })?;
    let status = res.status();
    let body = res.text().unwrap_or_default();
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("the team server said {status}"));
        return Err(Fail {
            conflict: status.as_u16() == 409,
            blocked: status.as_u16() == 402,
            msg,
        });
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

/// The code travels in a header, never the path — a logged URL is a leaked password.
fn fetch(t: &Team) -> Result<Doc, Fail> {
    call(
        client()
            .map_err(|msg| Fail { conflict: false, blocked: false, msg })?
            .get(format!("{}/team", base(&t.url)))
            .header("x-team", &t.code)
            .header("x-device", &t.device),
    )
}

fn put(t: &Team, doc: &str, version: i64) -> Result<Doc, Fail> {
    call(
        client()
            .map_err(|msg| Fail { conflict: false, blocked: false, msg })?
            .put(format!("{}/team", base(&t.url)))
            .header("x-team", &t.code)
            .header("x-device", &t.device)
            .json(&serde_json::json!({ "doc": doc, "version": version })),
    )
}

// ── what the window sees ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Status {
    /// off · synced · conflict · blocked · offline
    pub state: &'static str,
    pub url: String,
    pub code: String,
    pub seats: i64,
    pub paid: bool,
    pub error: Option<String>,
    /// The config file on this disk was rewritten, so whatever read it is stale.
    pub changed: bool,
}

fn off() -> Status {
    Status {
        state: "off",
        url: String::new(),
        code: String::new(),
        seats: 0,
        paid: false,
        error: None,
        changed: false,
    }
}

fn ok(t: &Team, d: &Doc, state: &'static str) -> Status {
    Status {
        state,
        url: t.url.clone(),
        code: t.code.clone(),
        seats: d.seats,
        paid: d.paid,
        error: None,
        changed: false,
    }
}

fn stuck(t: &Team, state: &'static str, msg: String) -> Status {
    Status {
        state,
        url: t.url.clone(),
        code: t.code.clone(),
        seats: 0,
        paid: false,
        error: Some(msg),
        changed: false,
    }
}

pub fn sync() -> Status {
    sync_at(&patchbay::config_path())
}

/// Fetch, then whichever of push or adopt applies. Idempotent, so every caller — the
/// window regaining focus, the end of an edit, the settings sheet — is this one call.
pub fn sync_at(cfg: &Path) -> Status {
    let Some(mut t) = load(cfg) else { return off() };
    let remote = match fetch(&t) {
        Ok(d) => d,
        Err(f) => return stuck(&t, "offline", f.msg),
    };

    let local = read_local(cfg);
    let we_moved = hash(&local) != t.synced;
    let they_moved = remote.version != t.version;

    match (we_moved, they_moved) {
        // Both sides moved, and only a person knows which one is right.
        (true, true) => stuck(&t, "conflict", "your list and the team's have both changed".into()),
        (true, false) => match put(&t, &shared(&local), t.version) {
            Ok(d) => {
                t.version = d.version;
                t.synced = hash(&local);
                match store(cfg, &t) {
                    Ok(()) => ok(&t, &d, "synced"),
                    Err(e) => stuck(&t, "offline", e),
                }
            }
            // Someone wrote between our fetch and our put — the next sync sees it as
            // the conflict it is, but say so now rather than reporting success.
            Err(f) if f.conflict => stuck(&t, "conflict", f.msg),
            Err(f) if f.blocked => stuck(&t, "blocked", f.msg),
            Err(f) => stuck(&t, "offline", f.msg),
        },
        (false, true) => match adopt(cfg, &mut t, &remote, &local) {
            Ok(()) => Status { changed: true, ..ok(&t, &remote, "synced") },
            Err(e) => stuck(&t, "offline", e),
        },
        (false, false) => ok(&t, &remote, "synced"),
    }
}

/// Take the team's document as ours, keeping this machine's `[settings]`.
fn adopt(cfg: &Path, t: &mut Team, remote: &Doc, local: &str) -> Result<(), String> {
    config::replace_at(cfg, &with_local_settings(&remote.doc, local))?;
    t.version = remote.version;
    // Hashed from the file as it now stands, not from what arrived: putting our
    // `[settings]` back and writing it out moves whitespace, and a hash of the wrong
    // one reads as a local edit and pushes the team's own list back at them.
    t.synced = hash(&read_local(cfg));
    store(cfg, t)
}

pub fn join(url: &str, code: &str) -> Result<Status, String> {
    join_at(&patchbay::config_path(), url, code)
}

pub fn join_at(cfg: &Path, url: &str, code: &str) -> Result<Status, String> {
    if !crate::is_web_url(url) {
        return Err("a team server address starts with http:// or https://".into());
    }
    let mut t = Team {
        url: base(url),
        code: code.trim().to_string(),
        // Rejoining from the same machine is the same seat, not a second one.
        device: load(cfg).map(|old| old.device).unwrap_or_else(new_device),
        version: 0,
        synced: String::new(),
    };
    if t.code.is_empty() {
        return Err("a team code, from whoever made the team".into());
    }
    let remote = fetch(&t).map_err(|f| f.msg)?;
    let local = read_local(cfg);

    // An empty team is one you just made, so your list becomes its list. Otherwise
    // the team's wins and yours is kept beside it: a merge is not ours to invent, and
    // losing a colleague's hosts is worse than either.
    if remote.doc.trim().is_empty() {
        let d = put(&t, &shared(&local), remote.version).map_err(|f| f.msg)?;
        t.version = d.version;
        t.synced = hash(&local);
        store(cfg, &t)?;
        return Ok(ok(&t, &d, "synced"));
    }
    if hash(&local) != hash(&remote.doc) {
        backup(cfg);
    }
    adopt(cfg, &mut t, &remote, &local)?;
    Ok(Status { changed: true, ..ok(&t, &remote, "synced") })
}

pub fn create(url: &str) -> Result<Status, String> {
    create_at(&patchbay::config_path(), url)
}

pub fn create_at(cfg: &Path, url: &str) -> Result<Status, String> {
    if !crate::is_web_url(url) {
        return Err("a team server address starts with http:// or https://".into());
    }
    let made: Code = call(client()?.post(format!("{}/teams", base(url))))
        .map_err(|f| f.msg)?;
    join_at(cfg, url, &made.code)
}

pub fn resolve(keep: &str) -> Result<Status, String> {
    resolve_at(&patchbay::config_path(), keep)
}

/// `mine` overwrites the team's copy, `theirs` overwrites ours — the two ways out of
/// a conflict, both of them somebody's deliberate choice, and both keeping a `.bak`.
pub fn resolve_at(cfg: &Path, keep: &str) -> Result<Status, String> {
    let mut t = load(cfg).ok_or("not in a team")?;
    let remote = fetch(&t).map_err(|f| f.msg)?;
    let local = read_local(cfg);

    if keep == "mine" {
        let d = put(&t, &shared(&local), remote.version).map_err(|f| f.msg)?;
        t.version = d.version;
        t.synced = hash(&local);
        store(cfg, &t)?;
        return Ok(ok(&t, &d, "synced"));
    }
    backup(cfg);
    adopt(cfg, &mut t, &remote, &local)?;
    Ok(Status { changed: true, ..ok(&t, &remote, "synced") })
}

pub fn leave() -> Result<(), String> {
    leave_at(&patchbay::config_path())
}

/// Leaving keeps the list — it is your config file, and it is already on this disk.
pub fn leave_at(cfg: &Path) -> Result<(), String> {
    let path = team_path(cfg);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    const LOCAL: &str = r#"# my hosts
[settings]
connect_in_terminal = true

[jack.web]
host = "10.0.0.4"
"#;

    #[test]
    fn settings_stay_on_this_machine_and_everything_else_travels() {
        let out = shared(LOCAL);
        assert!(!out.contains("[settings]"), "preferences went to the team:\n{out}");
        assert!(out.contains("[jack.web]"), "{out}");
        assert!(out.contains("# my hosts"), "comments travel with the document:\n{out}");

        // A teammate's document arrives without ours, and must not take our
        // preferences with it when it lands.
        let theirs = "[jack.db]\nhost = \"10.0.0.9\"\n";
        let merged = with_local_settings(theirs, LOCAL);
        assert!(merged.contains("connect_in_terminal = true"), "{merged}");
        assert!(merged.contains("[jack.db]"), "{merged}");
        assert!(!merged.contains("[jack.web]"), "the team's list is the list: {merged}");

        // No local settings is the normal case, and adds nothing.
        assert_eq!(with_local_settings(theirs, "[jack.x]\nhost = \"h\"\n"), theirs);
    }

    #[test]
    fn a_local_preference_is_not_a_change_worth_pushing() {
        let toggled = LOCAL.replace("connect_in_terminal = true", "connect_in_terminal = false");
        assert_eq!(hash(LOCAL), hash(&toggled), "flipping a local setting asked for a push");
        assert_ne!(hash(LOCAL), hash("[jack.web]\nhost = \"10.0.0.5\"\n"));
    }

    /// Stands in for `server/`: one document, one version, and a write that only
    /// lands if the version still matches. Close enough to pin our half of the
    /// contract — the headers, the conflict, and who overwrites whom — without a
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
                let doc_json = |d: &str, v: i64| {
                    format!(
                        "{{\"doc\":{},\"version\":{v},\"seats\":2,\"paid\":false}}",
                        serde_json::to_string(d).unwrap()
                    )
                };
                let (code, payload) = if !lower.contains("x-team:") || !lower.contains("x-device:") {
                    (400, "{\"error\":\"missing header\"}".to_string())
                } else if head.starts_with("GET") {
                    (200, doc_json(&st.0, st.1))
                } else {
                    let sent: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    if sent["version"].as_i64() != Some(st.1) {
                        (409, "{\"error\":\"the config changed underneath you\"}".to_string())
                    } else {
                        st.0 = sent["doc"].as_str().unwrap_or("").to_string();
                        st.1 += 1;
                        (200, doc_json(&st.0, st.1))
                    }
                };
                drop(st);
                let _ = write!(
                    sock,
                    "HTTP/1.1 {code} x\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        (url, state)
    }

    fn scratch(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("patchbay-team-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("patchbay.toml");
        std::fs::write(&cfg, body).unwrap();
        cfg
    }
    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    #[test]
    fn joining_an_empty_team_sends_your_list_and_a_later_edit_pushes() {
        let (url, server) = stub("");
        let cfg = scratch("empty", LOCAL);

        let s = join_at(&cfg, &url, "abcd-efgh").unwrap();
        assert_eq!(s.state, "synced", "{:?}", s.error);
        assert!(server.lock().unwrap().0.contains("[jack.web]"), "the list didn't go up");
        assert!(!server.lock().unwrap().0.contains("[settings]"), "preferences went up");

        // Nothing has changed on either side, so this neither pushes nor writes.
        let before = server.lock().unwrap().1;
        assert_eq!(sync_at(&cfg).state, "synced");
        assert_eq!(server.lock().unwrap().1, before, "an idle sync wrote anyway");

        std::fs::write(&cfg, format!("{LOCAL}\n[jack.db]\nhost = \"10.0.0.9\"\n")).unwrap();
        assert_eq!(sync_at(&cfg).state, "synced");
        assert!(server.lock().unwrap().0.contains("[jack.db]"), "the edit stayed here");
    }

    #[test]
    fn a_teammates_change_lands_here_and_leaves_our_preferences_alone() {
        let (url, server) = stub("");
        let cfg = scratch("pull", LOCAL);
        join_at(&cfg, &url, "abcd-efgh").unwrap();

        {
            let mut st = server.lock().unwrap();
            st.0 = "[jack.db]\nhost = \"10.0.0.9\"\n".into();
            st.1 += 1;
        }
        let s = sync_at(&cfg);
        assert_eq!(s.state, "synced", "{:?}", s.error);
        assert!(s.changed, "the window wasn't told the file moved");
        assert!(read(&cfg).contains("[jack.db]"), "{}", read(&cfg));
        assert!(!read(&cfg).contains("[jack.web]"), "the team's list is the list");
        assert!(read(&cfg).contains("connect_in_terminal = true"), "our preference went with it");

        // And once adopted it is the agreed document, so nothing bounces back.
        let version = server.lock().unwrap().1;
        assert_eq!(sync_at(&cfg).state, "synced");
        assert_eq!(server.lock().unwrap().1, version, "the pull pushed itself back");
    }

    #[test]
    fn both_sides_moving_is_a_conflict_until_someone_picks() {
        let (url, server) = stub("");
        let cfg = scratch("conflict", LOCAL);
        join_at(&cfg, &url, "abcd-efgh").unwrap();

        std::fs::write(&cfg, format!("{LOCAL}\n[jack.mine]\nhost = \"10.0.0.1\"\n")).unwrap();
        {
            let mut st = server.lock().unwrap();
            st.0 = "[jack.theirs]\nhost = \"10.0.0.2\"\n".into();
            st.1 += 1;
        }
        let s = sync_at(&cfg);
        assert_eq!(s.state, "conflict", "{:?}", s.error);
        assert!(read(&cfg).contains("[jack.mine]"), "a conflict overwrote the local file");

        // Taking theirs keeps ours beside it rather than dropping it.
        let s = resolve_at(&cfg, "theirs").unwrap();
        assert_eq!(s.state, "synced");
        assert!(read(&cfg).contains("[jack.theirs]"), "{}", read(&cfg));
        assert!(
            read(&cfg.with_extension("toml.bak")).contains("[jack.mine]"),
            "the list we dropped wasn't kept"
        );
        assert_eq!(sync_at(&cfg).state, "synced", "still in conflict after resolving");
    }

    #[test]
    fn pushing_mine_wins_the_conflict_the_other_way() {
        let (url, server) = stub("");
        let cfg = scratch("mine", LOCAL);
        join_at(&cfg, &url, "abcd-efgh").unwrap();

        std::fs::write(&cfg, format!("{LOCAL}\n[jack.mine]\nhost = \"10.0.0.1\"\n")).unwrap();
        server.lock().unwrap().1 += 1;
        assert_eq!(sync_at(&cfg).state, "conflict");

        assert_eq!(resolve_at(&cfg, "mine").unwrap().state, "synced");
        assert!(server.lock().unwrap().0.contains("[jack.mine]"));
        assert_eq!(sync_at(&cfg).state, "synced");
    }

    #[test]
    fn joining_a_team_that_already_has_a_list_keeps_yours_beside_it() {
        let (url, _server) = stub("[jack.theirs]\nhost = \"10.0.0.2\"\n");
        let cfg = scratch("takeover", LOCAL);

        assert_eq!(join_at(&cfg, &url, "abcd-efgh").unwrap().state, "synced");
        assert!(read(&cfg).contains("[jack.theirs]"), "{}", read(&cfg));
        assert!(read(&cfg.with_extension("toml.bak")).contains("[jack.web]"), "no backup kept");
    }

    #[test]
    fn no_team_means_no_request_at_all_and_a_dead_server_is_not_a_lost_list() {
        let cfg = scratch("off", LOCAL);
        assert_eq!(sync_at(&cfg).state, "off");
        assert!(leave_at(&cfg).is_ok(), "leaving when not in a team is not an error");

        // A port nothing listens on: the state says so and the file is untouched.
        store(
            &cfg,
            &Team {
                url: "http://127.0.0.1:1".into(),
                code: "abcd".into(),
                device: "d1".into(),
                version: 1,
                synced: String::new(),
            },
        )
        .unwrap();
        let s = sync_at(&cfg);
        assert_eq!(s.state, "offline");
        assert!(s.error.is_some());
        assert_eq!(read(&cfg), LOCAL);

        leave_at(&cfg).unwrap();
        assert_eq!(sync_at(&cfg).state, "off");
        assert_eq!(read(&cfg), LOCAL, "leaving took the list with it");
    }

    #[test]
    fn only_an_http_address_is_a_team_server() {
        let cfg = scratch("url", LOCAL);
        for bad in ["file:///etc/passwd", "ssh://box", "127.0.0.1:8787", ""] {
            assert!(join_at(&cfg, bad, "abcd").is_err(), "{bad:?} should be refused");
            assert!(create_at(&cfg, bad).is_err(), "{bad:?} should be refused");
        }
    }
}
