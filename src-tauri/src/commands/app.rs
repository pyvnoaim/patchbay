//! The app itself: version, self-update, quitting, and `patchbay://` links.

use super::load_jacks;
use crate::rdp;
use serde::Serialize;
use tauri::Manager;

#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// The close button is held by Rust and answered by the window, because only the
/// window knows which tabs are still live. This is the answer.
#[tauri::command]
pub fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

/// What the update pill offers.
#[derive(Serialize)]
pub struct Offer {
    version: String,
    /// The changelog section for that version. Not covered by the update signature,
    /// so the window escapes it like any outside text.
    notes: Option<String>,
}

/// The version waiting on the update endpoint, or `None` when this is the latest.
/// The plugin verifies the signature against the public key in `tauri.conf.json`.
#[tauri::command]
pub async fn update_check(app: tauri::AppHandle) -> Result<Option<Offer>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(found.map(|u| Offer {
        version: u.version,
        // Characters, not bytes: `String::truncate` panics mid-codepoint.
        notes: u.body.map(|b| b.chars().take(4000).collect()),
    }))
}

/// Download and swap the bundle in. Never restarts: that is `update_restart`, a second
/// click, because it takes every live session with it. Progress is emitted one whole
/// percent at a time; a server with no content-length sends none.
#[tauri::command]
pub async fn update_install(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;
    // Checked again rather than kept from `update_check`: an `Update` isn't `Send`.
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("that update is no longer offered")?;
    let feed = app.clone();
    let (mut got, mut last) = (0u64, 0u8);
    update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let Some(total) = total.filter(|t| *t > 0) else {
                    return;
                };
                let pct = (got * 100 / total).min(100) as u8;
                if pct != last {
                    last = pct;
                    let _ = feed.emit("update:progress", pct);
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn update_restart(app: tauri::AppHandle) {
    // A restart is not `RunEvent::Exit`, so the tunnels are closed by hand or
    // `ssh -N -L` outlives us and keeps the ports.
    app.state::<rdp::SharedTunnels>().close_all();
    app.restart();
}

/// A link that arrived before the window was listening. macOS launches the app to
/// deliver one, so on a cold start the first link lands while `boot.js` is still
/// loading, and `take_link` is how the window asks for it once it is.
#[derive(Default)]
pub struct PendingLink(std::sync::Mutex<Option<String>>);

#[tauri::command]
pub fn take_link(pending: tauri::State<'_, PendingLink>) -> Option<String> {
    pending.0.lock().unwrap().take()
}

/// Act on a `patchbay://` link: hold it for `take_link` and emit it for a window that
/// is already up. A link carries a device *name*, matched against the config exactly,
/// never an address; a name that isn't here is refused rather than guessed at.
pub fn deliver_link(handle: &tauri::AppHandle, urls: &[tauri::Url]) {
    use tauri::Emitter;
    let Some(name) = urls.iter().find_map(|u| link_target(u.as_str())) else {
        return;
    };
    *handle.state::<PendingLink>().0.lock().unwrap() = Some(name.clone());
    let _ = handle.emit("open:link", name);
}

/// The exact name, never `resolve`'s substring match: a link in an alert is acted on
/// unread, and `patchbay://db` quietly opening `db-prod` is the wrong box.
fn link_target(url: &str) -> Option<String> {
    let name = link_name(url)?;
    load_jacks().ok()?.contains_key(&name).then_some(name)
}

/// The name a link carries: the first path segment, and only if it can't be shaped
/// like `user@host:22`.
fn link_name(url: &str) -> Option<String> {
    let rest = url.strip_prefix("patchbay://")?.trim_end_matches('/');
    let name = percent_decode(rest.split(['/', '?', '#']).next()?);
    let ok = !name.is_empty()
        && !name.contains(['@', ':', ' ', '\\', '/'])
        && !name.chars().any(char::is_control);
    ok.then_some(name)
}

/// Bytes first, one decode at the end: `%C3%A9` is one letter, not two.
fn percent_decode(s: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut it = s.bytes().enumerate();
    while let Some((i, b)) = it.next() {
        if b == b'%' && s.len() > i + 2 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                it.next();
                it.next();
                continue;
            }
        }
        out.push(b);
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Eject the disk image the app was dragged out of. Nothing in macOS does it when the
/// drag finishes. Never while running *from* the image: that is someone trying the app
/// before installing it. `spawn`, not `status`: a volume Finder still has open simply
/// stays mounted, and that is not a startup's problem to report.
#[cfg(target_os = "macos")]
pub fn eject_install_image() {
    let volume = std::path::Path::new("/Volumes/patchbay");
    let ours = volume.join("patchbay.app").is_dir();
    let running_from_it = std::env::current_exe()
        .map(|p| p.starts_with(volume))
        .unwrap_or(true);
    if ours && !running_from_it {
        let _ = std::process::Command::new("/usr/bin/hdiutil")
            .args(["detach", "/Volumes/patchbay", "-quiet"])
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::link_name;

    /// A link is the one way into the app a web page can reach, so it may carry a
    /// device name and nothing that could dial a stranger's box.
    #[test]
    fn a_link_carries_a_device_name_and_never_a_login() {
        assert_eq!(link_name("patchbay://web-01").as_deref(), Some("web-01"));
        assert_eq!(link_name("patchbay://web-01/").as_deref(), Some("web-01"));
        assert_eq!(
            link_name("patchbay://db/etc/passwd?x=1").as_deref(),
            Some("db")
        );
        assert_eq!(link_name("patchbay://a%2Db").as_deref(), Some("a-b"));
        assert_eq!(link_name("patchbay://caf%C3%A9").as_deref(), Some("café"));
        assert_eq!(
            link_name("patchbay://web.example").as_deref(),
            Some("web.example")
        );

        for bad in [
            "patchbay://root@evil.example",
            "patchbay://10.0.0.1:22",
            "patchbay://",
            "patchbay://a b",
            "https://evil.example",
            "file:///etc/passwd",
        ] {
            assert_eq!(link_name(bad), None, "{bad:?} should carry no name");
        }
    }
}
