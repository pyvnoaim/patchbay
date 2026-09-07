//! The team licence: a signed blob, checked on this machine and nowhere else.
//!
//! Nothing is gated on it at the moment - the shared list is free, and `writable_list()`
//! in `commands/mod.rs` is where the wall stood. This is kept whole because the reasoning
//! is the part worth keeping: verification is deliberately offline, since an activation
//! check would be the only outbound request patchbay makes and "stores nothing, tells
//! nobody" is the whole argument for using it, and the check sits in public source where
//! it can be cut out, because it is a compliance mechanism for people who buy software
//! rather than a lock.

use crate::patchbay;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The public half of the licence key. The private half never leaves `~/.tauri`, and it
/// is deliberately *not* the updater's: a leaked licence key must not also mean signed
/// releases.
const PUBKEY: &str = "RWStHn7frty+GzohU0+g4sziMlpBap58+FVcC3cD8Pqj+LLeRs5+uSAu";

/// The line between the payload and its signature. The signature covers the payload
/// bytes exactly as they appear above this, so neither half is ever reformatted.
const MARK: &str = "\n--\n";

/// Beside the config, never in it: this is the machine's answer the way `web_trusted`
/// is, and a shared list must not carry one team's licence to everyone who reads it.
pub fn path() -> PathBuf {
    patchbay::config_path().with_file_name("licence")
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Licence {
    pub company: String,
    /// Unix seconds. A licence with no expiry never lapses.
    #[serde(default)]
    pub expires: Option<u64>,
}

impl Licence {
    pub fn lapsed(&self) -> bool {
        self.expires.is_some_and(|e| now() > e)
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

/// What is on disk, signature checked, whether or not it has lapsed.
pub fn read() -> Option<Licence> {
    read_at(&path())
}

pub fn read_at(p: &Path) -> Option<Licence> {
    parse(&std::fs::read_to_string(p).ok()?).ok()
}

/// Store a licence, or refuse it. Written whole rather than edited, so a bad paste
/// cannot leave half a licence behind.
pub fn save(blob: &str) -> Result<Licence, String> {
    let licence = parse(blob)?;
    if licence.lapsed() {
        return Err(format!(
            "that licence for \"{}\" has expired",
            licence.company
        ));
    }
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&p, blob.trim_start()).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(licence)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed with the real key by `npm run licence`, so this pins the whole chain: the
    /// script's framing, the payload bytes and the public key baked in above. Its expiry
    /// is deliberately in the past - the fixture ships in a public repo, and a working
    /// licence in one would unlock the very thing it guards.
    const GOOD: &str = include_str!("../../dev/licence.example");

    #[test]
    fn a_signed_licence_names_its_company() {
        let l = parse(GOOD).unwrap();
        assert_eq!(l.company, "Acme GmbH");
        assert!(l.lapsed(), "the fixture must never unlock anything");
    }

    #[test]
    fn an_edited_payload_is_refused() {
        let bad = GOOD.replace("Acme GmbH", "Acme Holdings");
        let err = parse(&bad).unwrap_err();
        assert!(err.contains("not signed for patchbay"), "{err}");
    }

    /// The expiry is signed too, so buying a year and editing it to ten is not a thing.
    #[test]
    fn an_edited_expiry_is_refused() {
        let line = GOOD.lines().find(|l| l.starts_with("expires")).unwrap();
        let bad = GOOD.replace(line, "expires = 4102444800");
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn something_that_is_not_a_licence_says_so() {
        let err = parse("hello").unwrap_err();
        assert!(err.contains("does not look like"), "{err}");
    }

    #[test]
    fn a_lapsed_licence_is_genuine_but_not_current() {
        assert!(parse(GOOD).is_ok());
        let dir = std::env::temp_dir().join(format!("patchbay-lic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("licence");
        std::fs::write(&p, GOOD).unwrap();
        assert!(read_at(&p).is_some_and(|l| l.lapsed()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `save()` is where someone finds out, rather than a licence sitting on disk that
    /// says a date that has been and gone.
    #[test]
    fn saving_a_lapsed_licence_says_it_expired() {
        let err = save(GOOD).unwrap_err();
        assert!(err.contains("expired"), "{err}");
        assert!(err.contains("Acme GmbH"), "{err}");
    }

    #[test]
    fn a_licence_with_no_expiry_never_lapses() {
        assert!(!Licence {
            company: "Acme GmbH".into(),
            expires: None,
        }
        .lapsed());
    }
}
