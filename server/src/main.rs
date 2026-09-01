//! patchbay's team server: one shared config document per team, and nothing else.
//!
//! It never parses the TOML. A team is `code -> { doc, version }`, so a new field in
//! the config needs no migration here and the server never learns what a jack is.
//!
//! There are no accounts. The team code *is* the credential, which matches the trust
//! model — everyone on a team sees everything — and removes the entire login layer.
//! Being the credential is also why it travels in `x-team` and never in the path: a
//! URL ends up in access logs, proxy logs and shell history, and a logged path is a
//! leaked password.
//!
//! Two limits worth knowing before this is anyone's billing model. Creating a team is
//! unauthenticated, so an open instance can be filled with empty teams. And a seat is
//! whatever a client calls itself: the count is honest only while clients are, and a
//! patched one that reuses a single device id never reaches the limit. Neither is
//! fixable here — both need an account or a signed build to mean anything.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use rand::Rng;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Enough for a team to feel it working before it costs anything.
const FREE_SEATS: i64 = 3;
/// A device silent for a month stops counting. Reinstalls and replaced laptops
/// shouldn't push a team over the line.
const SEAT_TTL: i64 = 30 * 24 * 3600;

/// ponytail: one connection behind a mutex. This is a document fetch per window
/// focus, not a workload — a pool when a team's traffic can be measured.
type Db = Arc<Mutex<Connection>>;

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

// ── errors ──────────────────────────────────────────────────────────────────

struct Fail(StatusCode, String);

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<rusqlite::Error> for Fail {
    fn from(e: rusqlite::Error) -> Self {
        Fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

// ── storage ─────────────────────────────────────────────────────────────────

fn open(path: &str) -> Connection {
    let db = Connection::open(path).expect("open database");
    db.execute_batch(
        "pragma journal_mode = wal;
         create table if not exists teams (
           code    text primary key,
           doc     text not null,
           version integer not null,
           paid    integer not null default 0,
           created integer not null
         );
         create table if not exists devices (
           code    text not null,
           device  text not null,
           seen    integer not null,
           primary key (code, device)
         );",
    )
    .expect("create schema");
    db
}

/// No vowels, so a code can't spell anything; no `0O1IL`, because people read these
/// out loud and retype them from a chat message.
const ALPHABET: &[u8] = b"23456789bcdfghjkmnpqrstvwxyz";

fn new_code() -> String {
    let mut rng = rand::thread_rng();
    let raw: Vec<u8> = (0..16)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
        .collect();
    raw.chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

/// `x-team` is the code, `x-device` is which machine is asking. Both are required and
/// neither is guessed at: seats are counted from the device, and an unattributable
/// write is a write that never counts against the limit.
fn header(headers: &HeaderMap, name: &str) -> Result<String, Fail> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .map(str::to_owned)
        .ok_or_else(|| Fail(StatusCode::BAD_REQUEST, format!("missing {name} header")))
}

fn touch(db: &Connection, code: &str, dev: &str) -> Result<(), Fail> {
    db.execute(
        "insert into devices (code, device, seen) values (?1, ?2, ?3)
         on conflict (code, device) do update set seen = ?3",
        (code, dev, now()),
    )?;
    Ok(())
}

fn seats(db: &Connection, code: &str) -> Result<i64, Fail> {
    Ok(db.query_row(
        "select count(*) from devices where code = ?1 and seen > ?2",
        (code, now() - SEAT_TTL),
        |r| r.get(0),
    )?)
}

// ── handlers ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Created {
    code: String,
}

async fn create_team(State(db): State<Db>) -> Result<Json<Created>, Fail> {
    let code = new_code();
    db.lock().unwrap().execute(
        "insert into teams (code, doc, version, created) values (?1, '', 1, ?2)",
        (&code, now()),
    )?;
    Ok(Json(Created { code }))
}

#[derive(Serialize)]
struct Doc {
    doc: String,
    version: i64,
    seats: i64,
    paid: bool,
}

fn team(db: &Connection, code: &str) -> Result<(String, i64, bool), Fail> {
    db.query_row(
        "select doc, version, paid from teams where code = ?1",
        [code],
        |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
    )
    .optional()?
    // The code is not quoted back, unlike every other error here: it is the
    // credential, and an error body is exactly what a client writes to a log.
    .ok_or_else(|| Fail(StatusCode::NOT_FOUND, "no team with that code".into()))
}

/// Reading stays open past the seat limit on purpose. Locking a team out of its own
/// device list is exactly the wrong thing to do during an outage; writes are where
/// the value is, so that's where the paywall goes.
async fn get_doc(State(db): State<Db>, headers: HeaderMap) -> Result<Json<Doc>, Fail> {
    let code = header(&headers, "x-team")?;
    let dev = header(&headers, "x-device")?;
    let db = db.lock().unwrap();
    let (doc, version, paid) = team(&db, &code)?;
    touch(&db, &code, &dev)?;
    Ok(Json(Doc { doc, version, seats: seats(&db, &code)?, paid }))
}

#[derive(Deserialize)]
struct Put {
    doc: String,
    version: i64,
}

async fn put_doc(
    State(db): State<Db>,
    headers: HeaderMap,
    Json(body): Json<Put>,
) -> Result<Json<Doc>, Fail> {
    let code = header(&headers, "x-team")?;
    let dev = header(&headers, "x-device")?;
    let db = db.lock().unwrap();
    let (_, version, paid) = team(&db, &code)?;
    touch(&db, &code, &dev)?;

    // Optimistic concurrency: whoever writes second re-fetches and re-applies. The
    // app changes one field at a time through toml_edit, so a retry is cheap and
    // there is no merge algorithm to get wrong.
    if body.version != version {
        return Err(Fail(
            StatusCode::CONFLICT,
            "the config changed underneath you — fetch it again".into(),
        ));
    }
    let count = seats(&db, &code)?;
    if !paid && count > FREE_SEATS {
        return Err(Fail(
            StatusCode::PAYMENT_REQUIRED,
            format!("{count} people on a team of {FREE_SEATS} — everyone can still read it"),
        ));
    }

    let next = version + 1;
    db.execute(
        "update teams set doc = ?1, version = ?2 where code = ?3",
        (&body.doc, next, &code),
    )?;
    Ok(Json(Doc { doc: body.doc, version: next, seats: count, paid }))
}

fn app(db: Db) -> Router {
    Router::new()
        .route("/teams", post(create_team))
        .route("/team", axum::routing::get(get_doc).put(put_doc))
        .with_state(db)
}

#[tokio::main]
async fn main() {
    let path = std::env::var("PATCHBAY_DB").unwrap_or_else(|_| "patchbay.db".into());
    let addr = std::env::var("PATCHBAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8787".into());

    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("patchbay server on {addr}, database {path}");
    axum::serve(listener, app(Arc::new(Mutex::new(open(&path)))))
        .await
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn db() -> Db {
        Arc::new(Mutex::new(open(":memory:")))
    }

    async fn call(db: &Db, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let res = app(db.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    /// An empty `team` sends no `x-team` header — creating a team is the one call
    /// that has no code yet.
    fn req(
        method: &str,
        uri: &str,
        dev: &str,
        team: &str,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri).header("x-device", dev);
        if !team.is_empty() {
            b = b.header("x-team", team);
        }
        match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    }

    async fn a_team(db: &Db) -> String {
        let (status, v) = call(db, req("POST", "/teams", "d1", "", None)).await;
        assert_eq!(status, StatusCode::OK);
        v["code"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_new_team_starts_empty_and_an_unknown_code_is_not_found() {
        let db = db();
        let code = a_team(&db).await;

        let (status, v) = call(&db, req("GET", "/team", "d1", &code, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["doc"], "");
        assert_eq!(v["version"], 1);

        let (status, v) = call(&db, req("GET", "/team", "d1", "nope", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // The code is a credential, so the error names the failure without repeating it.
        let err = v["error"].as_str().unwrap();
        assert!(err.contains("no team"), "got {v}");
        assert!(!err.contains("nope"), "the code came back in the error: {v}");
    }

    #[tokio::test]
    async fn a_write_lands_and_a_stale_one_is_told_to_refetch() {
        let db = db();
        let code = a_team(&db).await;
        let doc = "[jack.web]\nhost = \"10.0.0.4\"\n";

        let body = serde_json::json!({ "doc": doc, "version": 1 });
        let (status, v) = call(&db, req("PUT", "/team", "d1", &code, Some(body.clone()))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["version"], 2);

        // The teammate who still thinks it's version 1 gets refused, not silently
        // overwritten — and the document on the server is untouched.
        let (status, _) = call(&db, req("PUT", "/team", "d2", &code, Some(body))).await;
        assert_eq!(status, StatusCode::CONFLICT);

        let (_, v) = call(&db, req("GET", "/team", "d2", &code, None)).await;
        assert_eq!(v["doc"], doc);
        assert_eq!(v["version"], 2);
    }

    #[tokio::test]
    async fn a_fourth_seat_stops_writing_but_everyone_can_still_read() {
        let db = db();
        let code = a_team(&db).await;
        for d in ["d1", "d2", "d3"] {
            let (status, _) = call(&db, req("GET", "/team", d, &code, None)).await;
            assert_eq!(status, StatusCode::OK);
        }
        let body = serde_json::json!({ "doc": "host = \"x\"", "version": 1 });
        let (status, _) = call(&db, req("PUT", "/team", "d3", &code, Some(body))).await;
        assert_eq!(status, StatusCode::OK, "three seats are free");

        let (status, v) = call(&db, req("PUT", "/team", "d4", &code, Some(serde_json::json!({ "doc": "", "version": 2 })))).await;
        assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
        assert!(v["error"].as_str().unwrap().contains("still read"), "got {v}");

        let (status, v) = call(&db, req("GET", "/team", "d4", &code, None)).await;
        assert_eq!(status, StatusCode::OK, "a paywall must not lock a team out of its own list");
        assert_eq!(v["seats"], 4);
    }

    #[tokio::test]
    async fn a_paid_team_writes_past_the_free_limit() {
        let db = db();
        let code = a_team(&db).await;
        for d in ["d1", "d2", "d3", "d4"] {
            call(&db, req("GET", "/team", d, &code, None)).await;
        }
        db.lock()
            .unwrap()
            .execute("update teams set paid = 1 where code = ?1", [&code])
            .unwrap();

        let body = serde_json::json!({ "doc": "host = \"x\"", "version": 1 });
        let (status, _) = call(&db, req("PUT", "/team", "d4", &code, Some(body))).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_write_without_a_device_is_refused() {
        let db = db();
        let code = a_team(&db).await;
        let r = Request::builder()
            .method("GET")
            .uri("/team")
            .header("x-team", &code)
            .body(Body::empty())
            .unwrap();
        let (status, _) = call(&db, r).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// The code only ever arrives in a header now, so a request that leaves it out is
    /// refused rather than falling back to anything in the path.
    #[tokio::test]
    async fn a_request_without_a_team_header_is_refused() {
        let db = db();
        let code = a_team(&db).await;
        let (status, _) = call(&db, req("GET", "/team", "d1", "", None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = call(&db, req("GET", &format!("/team/{code}"), "d1", "", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "no route carries the code any more");
    }

    #[test]
    fn codes_are_unambiguous_and_hard_to_guess() {
        let a = new_code();
        assert_eq!(a.len(), 19, "four groups of four: {a}");
        assert_ne!(a, new_code());
        assert!(
            !a.chars().any(|c| "aeiou01il".contains(c)),
            "nothing to misread or misspell: {a}"
        );
    }
}
