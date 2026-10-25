//! The team licence: a signed file beside the shared list, checked on this machine and
//! nowhere else.
//!
//! patchbay is free for one person. A team on a shared list needs a company licence,
//! and the shared list opens while one runs. Your own devices always open: the licence
//! is about the list a company shares, never about one person's work. Verification is
//! offline on purpose - an activation check would be a second outbound request, and
//! "tells nobody" is the argument for using patchbay at all - and it sits in public
//! source where it can be cut out, because it is a compliance mechanism for companies
//! that buy software rather than a lock.
//!
//! `until` is held against today's date. A licence that has ended gives `GRACE` more
//! days, the list still opening under a "licence ended" line, so nobody loses their
//! devices in the middle of an incident; after that the list stops opening. None of that
//! applies until `ON_SALE`: before the store opens there is nothing to buy, so every
//! shared list opens and the window asks for nothing. `ON_SALE` turns on in the release
//! that opens the store, and nowhere else.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Whether a licence is needed yet. False until the store opens; see the header.
pub const ON_SALE: bool = false;

/// How long a list keeps opening after its licence ends.
pub const GRACE: u64 = 7 * 86_400;

/// The public half of the licence key. The private half never leaves `~/.tauri`, and it
/// is deliberately *not* the updater's: a leaked licence key must not also mean signed
/// releases.
const PUBKEY: &str = "RWStHn7frty+GzohU0+g4sziMlpBap58+FVcC3cD8Pqj+LLeRs5+uSAu";

/// The line between the payload and its signature. The signature covers the payload
/// bytes exactly as they appear above this, so neither half is ever reformatted.
const MARK: &str = "\n--\n";

/// A licence is a few hundred bytes, and the file is on a share anyone can write.
const MAX: u64 = 4096;

/// Beside the shared list, like the team's host keys: a company licence covers the
/// team, so one paste is everyone's, and a list that moves takes its licence along.
pub fn path(list: &Path) -> PathBuf {
    list.with_file_name("patchbay_licence")
}

#[derive(Debug, Deserialize)]
pub struct Licence {
    pub company: String,
    /// Unix seconds: the end of the last day it runs. None for one that never ends.
    /// `updates_until` is what licences signed before the date meant today were called.
    #[serde(default, alias = "updates_until")]
    pub until: Option<u64>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Status {
    Unlicensed,
    Licensed {
        company: String,
        until: Option<u64>,
    },
    /// Ended, inside the days `GRACE` gives; `closes` is when they run out.
    Grace {
        company: String,
        until: u64,
        closes: u64,
    },
    Ended {
        company: String,
        until: u64,
    },
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Check a pasted licence and read what it says.
pub fn parse(blob: &str) -> Result<Licence, String> {
    let blob = blob.trim_start();
    let cut = blob
        .find(MARK)
        .ok_or("that does not look like a patchbay licence")?;
    // The newline belongs to the payload: it is in the bytes that were signed.
    let payload = &blob[..=cut];
    let signature = minisign_verify::Signature::decode(blob[cut + MARK.len()..].trim())
        .map_err(|_| "the licence signature cannot be read".to_string())?;
    minisign_verify::PublicKey::from_base64(PUBKEY)
        .map_err(|e| format!("the built-in licence key is unusable: {e}"))?
        .verify(payload.as_bytes(), &signature, false)
        .map_err(|_| "that licence was not signed for patchbay".to_string())?;
    toml::from_str(payload).map_err(|e| format!("the licence is signed but unreadable: {e}"))
}

pub fn status_of(l: Option<Licence>, now: u64) -> Status {
    let Some(Licence { company, until }) = l else {
        return Status::Unlicensed;
    };
    match until {
        Some(until) if now > until.saturating_add(GRACE) => Status::Ended { company, until },
        Some(until) if now > until => Status::Grace {
            company,
            until,
            closes: until + GRACE,
        },
        until => Status::Licensed { company, until },
    }
}

/// What the file beside `list` says. Missing, too big or not signed by us are all the
/// same answer: there is no licence.
pub fn status(list: &Path) -> Status {
    status_of(read_at(&path(list)), now())
}

/// Whether a shared list in this standing opens.
pub fn opens(s: &Status) -> bool {
    !ON_SALE || matches!(s, Status::Licensed { .. } | Status::Grace { .. })
}

/// Why a shared list didn't open, for the line where its devices would be.
pub fn refusal(s: &Status) -> String {
    match s {
        Status::Ended { company, until } => format!(
            "{company}'s licence ended on {}, so the team's list is closed: Settings ▸ Licence",
            day(*until)
        ),
        _ => "the team's list needs a company licence: Settings ▸ Licence".into(),
    }
}

/// A licence's day as `2027-10-20`, in UTC, the zone `npm run licence` signs in. Howard
/// Hinnant's days-to-civil, rather than a date crate for one line of text.
pub fn day(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn read_at(p: &Path) -> Option<Licence> {
    let mut text = String::new();
    std::fs::File::open(p)
        .ok()?
        .take(MAX + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX {
        return None;
    }
    parse(&text).ok()
}

/// Store a licence for the team, or refuse it. Written whole rather than edited, so a
/// bad paste can't leave half a licence behind.
pub fn save(list: &Path, blob: &str) -> Result<Status, String> {
    let status = status_of(Some(parse(blob)?), now());
    if let Status::Ended { company, until } = &status {
        return Err(format!(
            "that licence for \"{company}\" ended on {}",
            day(*until)
        ));
    }
    let p = path(list);
    std::fs::write(&p, blob.trim_start()).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(status)
}

pub fn remove(list: &Path) -> Result<(), String> {
    match std::fs::remove_file(path(list)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{e}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed with the real key by `npm run licence`, so this pins the whole chain: the
    /// script's framing, the payload bytes and the public key baked in above. It ended in
    /// 2020 - the fixture ships in a public repo, and one that still ran would license
    /// every team that copied it.
    const GOOD: &str = include_str!("../../dev/licence.example");
    const UNTIL: u64 = 1_577_836_799;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("patchbay-lic-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("patchbay.toml")
    }

    #[test]
    fn a_signed_licence_names_its_company_and_its_day() {
        let l = parse(GOOD).unwrap();
        assert_eq!(l.company, "Acme GmbH");
        assert_eq!(l.until, Some(UNTIL));
        assert_eq!(day(UNTIL), "2019-12-31");
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(951_782_400), "2000-02-29");
    }

    #[test]
    fn a_licence_runs_then_gives_a_week_then_ends() {
        let at = |now| status_of(parse(GOOD).ok(), now);
        assert!(matches!(at(UNTIL), Status::Licensed { .. }));
        assert!(matches!(at(UNTIL + 1), Status::Grace { .. }));
        assert!(matches!(at(UNTIL + GRACE), Status::Grace { .. }));
        assert!(matches!(at(UNTIL + GRACE + 1), Status::Ended { .. }));
        assert!(
            matches!(at(now()), Status::Ended { .. }),
            "the fixture must never license anyone today"
        );
        let forever = Licence {
            company: "Acme GmbH".into(),
            until: None,
        };
        assert!(matches!(
            status_of(Some(forever), u64::MAX),
            Status::Licensed { .. }
        ));
    }

    #[test]
    fn a_list_opens_while_a_licence_runs_once_they_are_on_sale() {
        let ended = Status::Ended {
            company: "Acme GmbH".into(),
            until: UNTIL,
        };
        let grace = Status::Grace {
            company: "Acme GmbH".into(),
            until: UNTIL,
            closes: UNTIL + GRACE,
        };
        // Before the store opens there is nothing to buy, so everything opens.
        assert_eq!(opens(&Status::Unlicensed), !ON_SALE);
        assert_eq!(opens(&ended), !ON_SALE);
        assert!(opens(&grace));
        assert!(refusal(&ended).contains("2019-12-31"));
        assert!(refusal(&Status::Unlicensed).contains("Settings ▸ Licence"));
    }

    /// A licence fresh from `npm run licence`, checked by the app: the script and the key
    /// still agree. `PATCHBAY_LICENCE=/path cargo test fresh -- --ignored`
    #[test]
    #[ignore = "needs a licence signed with the private key"]
    fn a_fresh_licence_from_the_script_runs() {
        let p = std::env::var("PATCHBAY_LICENCE").expect("PATCHBAY_LICENCE");
        let l = parse(&std::fs::read_to_string(p).unwrap()).expect("verifies");
        println!("{} until {:?}", l.company, l.until.map(day));
        assert!(matches!(status_of(Some(l), now()), Status::Licensed { .. }));
    }

    #[test]
    fn an_edited_payload_is_refused() {
        let bad = GOOD.replace("Acme GmbH", "Acme Holdings");
        let err = parse(&bad).unwrap_err();
        assert!(err.contains("not signed for patchbay"), "{err}");
    }

    /// The date is signed too, so buying a year and editing it to ten is not a thing.
    #[test]
    fn an_edited_date_is_refused() {
        let line = GOOD
            .lines()
            .find(|l| l.starts_with("updates_until"))
            .unwrap();
        let bad = GOOD.replace(line, "updates_until = 4102444800");
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn something_that_is_not_a_licence_says_so() {
        let err = parse("hello").unwrap_err();
        assert!(err.contains("does not look like"), "{err}");
    }

    #[test]
    fn a_licence_beside_the_list_is_read_and_a_missing_or_huge_one_is_none() {
        let list = scratch("read");
        assert_eq!(status(&list), Status::Unlicensed);
        std::fs::write(path(&list), GOOD).unwrap();
        assert!(matches!(status(&list), Status::Ended { .. }));
        std::fs::write(path(&list), format!("{GOOD}{}", " ".repeat(MAX as usize))).unwrap();
        assert_eq!(status(&list), Status::Unlicensed);
        remove(&list).unwrap();
        remove(&list).unwrap();
        std::fs::remove_dir_all(list.parent().unwrap()).unwrap();
    }

    /// `save()` is where someone finds out, rather than a licence sitting beside the list
    /// that opens nothing.
    #[test]
    fn saving_an_ended_licence_says_so_and_writes_nothing() {
        let list = scratch("save");
        let err = save(&list, GOOD).unwrap_err();
        assert!(err.contains("ended on 2019-12-31"), "{err}");
        assert!(err.contains("Acme GmbH"), "{err}");
        assert!(!path(&list).exists());
        std::fs::remove_dir_all(list.parent().unwrap()).unwrap();
    }
}
