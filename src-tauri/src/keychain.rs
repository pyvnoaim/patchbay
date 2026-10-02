//! RDP sign-ins remembered on this machine, in the OS keychain - never in the list,
//! which a colleague may be reading, and never in a file beside it, which would need a
//! key kept beside it too. Only ever ticked into, one device at a time, and a
//! device can hold several accounts: the admin and the user it is being fixed for.
//!
//! Filed under the device's own `host:port` (the same pin its certificate has), not its
//! name: a shared list edited to point `win01` somewhere else must not carry the
//! password along to wherever that is.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const SERVICE: &str = "patchbay rdp";

/// Every account this machine signs in to `pin` as, most recently saved first.
type Saved = Vec<(String, String)>;

/// Every read is held for the run. On macOS an ad-hoc signed app is a stranger to the
/// keychain after each update, and a read is a prompt until "Always Allow".
static READ: LazyLock<Mutex<HashMap<String, Saved>>> = LazyLock::new(Default::default);

fn entry(pin: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, pin).map_err(|e| format!("the keychain: {e}"))
}

/// One keychain item per desktop holding every account, so listing them is one read
/// and one prompt rather than one per account.
pub fn get(pin: &str) -> Result<Saved, String> {
    if let Some(hit) = READ.lock().unwrap().get(pin) {
        return Ok(hit.clone());
    }
    let raw = match entry(pin)?.get_password() {
        Ok(raw) => raw,
        Err(keyring::Error::NoEntry) => return Ok(Vec::new()),
        Err(e) => return Err(format!("the keychain: {e}")),
    };
    let saved = parse(&raw);
    READ.lock().unwrap().insert(pin.to_string(), saved.clone());
    Ok(saved)
}

/// A list, or the single pair 0.1.25 saved. Something else in the keychain under our
/// name reads as nothing saved.
fn parse(raw: &str) -> Saved {
    serde_json::from_str::<Saved>(raw)
        .or_else(|_| serde_json::from_str::<(String, String)>(raw).map(|p| vec![p]))
        .unwrap_or_default()
}

/// Windows accounts don't care about case, so neither does finding one.
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub fn password(pin: &str, user: &str) -> Result<Option<String>, String> {
    Ok(get(pin)?
        .into_iter()
        .find(|(u, _)| same(u, user))
        .map(|(_, p)| p))
}

fn put(pin: &str, saved: Saved) -> Result<(), String> {
    if saved.is_empty() {
        return forget(pin);
    }
    let raw = serde_json::to_string(&saved).map_err(|e| e.to_string())?;
    entry(pin)?
        .set_password(&raw)
        .map_err(|e| format!("the keychain: {e}"))?;
    READ.lock().unwrap().insert(pin.to_string(), saved);
    Ok(())
}

pub fn set(pin: &str, user: &str, password: &str) -> Result<(), String> {
    let mut saved = get(pin)?;
    saved.retain(|(u, _)| !same(u, user));
    saved.insert(0, (user.to_string(), password.to_string()));
    put(pin, saved)
}

pub fn forget_user(pin: &str, user: &str) -> Result<(), String> {
    let mut saved = get(pin)?;
    let before = saved.len();
    saved.retain(|(u, _)| !same(u, user));
    if saved.len() == before {
        return Ok(());
    }
    put(pin, saved)
}

/// Every account saved for this desktop.
pub fn forget(pin: &str) -> Result<(), String> {
    READ.lock().unwrap().remove(pin);
    match entry(pin)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("the keychain: {e}")),
    }
}

/// Whether there is a keychain to put anything in: a Linux desktop without a Secret
/// Service running has none, and the window hides the tick box rather than offer it.
pub fn available() -> bool {
    static AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
        entry("probe")
            .is_ok_and(|e| matches!(e.get_password(), Ok(_) | Err(keyring::Error::NoEntry)))
    });
    *AVAILABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_saved_pair_still_reads() {
        let one = parse(r#"["admin","pw"]"#);
        assert_eq!(one, vec![("admin".into(), "pw".into())]);
        let two = parse(r#"[["admin","pw"],["CORP\\bob","x"]]"#);
        assert_eq!(two.len(), 2);
        assert_eq!(two[1].0, "CORP\\bob");
        assert!(parse("not ours").is_empty());
    }
}
