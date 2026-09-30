//! RDP sign-ins remembered on this machine, in the OS keychain - never in the list,
//! which a colleague may be reading, and never in a file beside it, which would need a
//! key kept beside it too. Only ever ticked into, one device at a time.
//!
//! Filed under the device's own `host:port` (the same pin its certificate has), not its
//! name: a shared list edited to point `win01` somewhere else must not carry the
//! password along to wherever that is.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const SERVICE: &str = "patchbay rdp";

/// Every read is held for the run. On macOS an ad-hoc signed app is a stranger to the
/// keychain after each update, and a read is a prompt until "Always Allow".
static READ: LazyLock<Mutex<HashMap<String, (String, String)>>> = LazyLock::new(Default::default);

fn entry(pin: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, pin).map_err(|e| format!("the keychain: {e}"))
}

pub fn get(pin: &str) -> Result<Option<(String, String)>, String> {
    if let Some(hit) = READ.lock().unwrap().get(pin) {
        return Ok(Some(hit.clone()));
    }
    let raw = match entry(pin)?.get_password() {
        Ok(raw) => raw,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(format!("the keychain: {e}")),
    };
    // Something else in the keychain under our name reads as nothing saved.
    let Ok(pair) = serde_json::from_str::<(String, String)>(&raw) else {
        return Ok(None);
    };
    READ.lock().unwrap().insert(pin.to_string(), pair.clone());
    Ok(Some(pair))
}

pub fn set(pin: &str, user: &str, password: &str) -> Result<(), String> {
    let raw = serde_json::to_string(&(user, password)).map_err(|e| e.to_string())?;
    entry(pin)?
        .set_password(&raw)
        .map_err(|e| format!("the keychain: {e}"))?;
    READ.lock()
        .unwrap()
        .insert(pin.to_string(), (user.to_string(), password.to_string()));
    Ok(())
}

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
