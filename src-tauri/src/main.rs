#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod patchbay;
mod pty;
mod rdp;
mod terminal;
mod vpn;

use serde::Serialize;
use tauri::Manager;

#[derive(Serialize)]
struct JackView {
    name: String,
    host: String,
    user: Option<String>,
    port: Option<u16>,
    key: Option<String>,
    jump: Option<String>,
    os: Option<String>,
    url: Option<String>,
    rdp: Option<u16>,
    ssh: bool,
    primary: String,
    desc: Option<String>,
    tags: Vec<String>,
    forward: Vec<String>,
    /// Ordered hops, first one nearest us — what the detail pane draws as the route.
    hops: Vec<String>,
    command: String,
}

#[derive(Serialize)]
struct Probe {
    name: String,
    target: String,
    /// TCP connect time in ms, or null if it didn't answer inside the timeout.
    ms: Option<u64>,
}

fn read() -> Result<patchbay::Jacks, String> {
    let path = patchbay::config_path();
    // No config is not an error in the window — it's a first run, and the UI has
    // somewhere to put that. A config that exists but won't parse still is one.
    if !path.exists() {
        return Ok(patchbay::Jacks::new());
    }
    patchbay::load(&path)
}

#[tauri::command]
fn jacks() -> Result<Vec<JackView>, String> {
    let jacks = read()?;
    Ok(jacks
        .iter()
        .map(|(name, j)| JackView {
            name: name.clone(),
            host: j.host.clone(),
            user: j.user.clone(),
            port: j.port,
            jump: j.jump.clone(),
            os: j.os.clone(),
            url: j.url.clone(),
            rdp: j.rdp,
            ssh: j.ssh.unwrap_or(true),
            // Resolved here so both the window and any future front end agree.
            primary: {
                let has = |k: &str| match k {
                    "ssh" => j.ssh.unwrap_or(true),
                    "rdp" => j.rdp.is_some(),
                    _ => j.url.is_some(),
                };
                j.primary
                    .as_deref()
                    .filter(|p| has(p))
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        ["ssh", "rdp", "web"].iter().find(|k| has(k)).unwrap_or(&"ssh").to_string()
                    })
            },
            desc: j.desc.clone(),
            key: j.key.clone(),
            tags: j.tags.clone().unwrap_or_default(),
            forward: j.forward.clone().unwrap_or_default(),
            hops: patchbay::hops(name, &jacks).unwrap_or_default(),
            // Shown in the detail pane, so you always see what you're about to run.
            command: patchbay::ssh_args(name, &jacks)
                .map(|a| terminal::command_line(&a))
                .unwrap_or_else(|e| e),
        })
        .collect())
}

/// TCP-connect every jack's entry point in parallel. Nothing is sent; the socket is
/// opened and dropped. ponytail: one thread per jack, fine to a few hundred — swap for
/// a bounded pool if someone shows up with a thousand.
#[tauri::command]
async fn probe() -> Result<Vec<Probe>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let jacks = read()?;
        let targets: Vec<(String, String, u16)> = jacks
            .keys()
            .filter_map(|n| patchbay::entry(n, &jacks).ok().map(|(h, p)| (n.clone(), h, p)))
            .collect();

        Ok(std::thread::scope(|s| {
            let handles: Vec<_> = targets
                .iter()
                .map(|(name, host, port)| {
                    s.spawn(move || Probe {
                        name: name.clone(),
                        target: format!("{host}:{port}"),
                        ms: connect_ms(host, *port),
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn connect_ms(host: &str, port: u16) -> Option<u64> {
    use std::net::ToSocketAddrs;
    let started = std::time::Instant::now();
    let addr = format!("{host}:{port}").to_socket_addrs().ok()?.next()?;
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)).ok()?;
    Some(started.elapsed().as_millis() as u64)
}

#[tauri::command]
fn connect(name: String) -> Result<String, String> {
    let jacks = read()?;
    let args = patchbay::ssh_args(&patchbay::resolve(&name, &jacks)?, &jacks)?;
    terminal::open(&args)?;
    Ok(terminal::command_line(&args))
}

/// Opens a session in the window. `connect` is still there for "open in my real
/// terminal" — this is the in-app one.
#[tauri::command]
fn open_session(
    app: tauri::AppHandle,
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let jacks = read()?;
    let resolved = patchbay::resolve(&name, &jacks)?;
    let args = patchbay::ssh_args(&resolved, &jacks)?;
    sessions.open(&app, id, &args, cols.max(2), rows.max(2))?;
    Ok(terminal::command_line(&args))
}

#[tauri::command]
fn write_session(sessions: tauri::State<'_, pty::Shared>, id: u32, data: String) -> Result<(), String> {
    sessions.write(id, &data)
}

#[tauri::command]
fn resize_session(sessions: tauri::State<'_, pty::Shared>, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    sessions.resize(id, cols.max(2), rows.max(2))
}

#[tauri::command]
fn close_session(sessions: tauri::State<'_, pty::Shared>, id: u32) {
    sessions.close(id);
}

#[tauri::command]
fn save_jack(original: Option<String>, jack: config::JackInput) -> Result<(), String> {
    config::save_jack(original, jack)
}

#[tauri::command]
fn delete_jack(name: String) -> Result<(), String> {
    config::delete_jack(&name)
}

#[tauri::command]
fn rename_group(from: String, to: String) -> Result<usize, String> {
    config::rename_group(&from, &to)
}

#[tauri::command]
fn delete_group(path: String) -> Result<usize, String> {
    config::delete_group(&path)
}

/// Only http(s) may be handed to the desktop. `open`/`explorer` will happily launch
/// an application for a custom scheme, and the config is a file people share.
pub fn is_web_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://"))
        && !u.contains(['\n', '\r', '\0'])
}

/// Hand a path or URL to the desktop's default handler. Deliberately not through a
/// shell: `cmd /c start` would re-read `&` in a URL as a command separator.
fn os_open(arg: &std::ffi::OsStr) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(arg)
        .spawn()
        .map_err(|e| format!("could not open {}: {e}", arg.to_string_lossy()))?;
    Ok(())
}

/// Opens a jack's web UI in the real browser — the same handoff rule as RDP.
#[tauri::command]
fn open_url(name: String) -> Result<String, String> {
    let jacks = read()?;
    let resolved = patchbay::resolve(&name, &jacks)?;
    let url = jacks
        .get(&resolved)
        .and_then(|j| j.url.as_deref())
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| format!("\"{resolved}\" has no url"))?
        .to_string();
    if !is_web_url(&url) {
        return Err("only http:// and https:// urls can be opened".into());
    }
    os_open(url.as_ref())?;
    Ok(url)
}

/// Remembered state for VPNs with no `check` command — best effort, and the UI
/// says so rather than pretending it measured anything.
#[derive(Default)]
struct VpnState(std::sync::Mutex<std::collections::HashMap<String, bool>>);

#[tauri::command]
async fn vpns(state: tauri::State<'_, std::sync::Arc<VpnState>>) -> Result<Vec<vpn::VpnView>, String> {
    let remembered = state.0.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let defs = vpn::load(&patchbay::config_path())?;
        Ok(defs
            .iter()
            .map(|(path, v)| match vpn::is_up(v) {
                Some(up) => vpn::VpnView { path: path.clone(), up, known: true },
                None => vpn::VpnView {
                    path: path.clone(),
                    up: *remembered.get(path).unwrap_or(&false),
                    known: false,
                },
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Runs the folder's `up` or `down`. Whatever you put there is what runs — see the
/// warning in the README; this is the one place the config is more than an ssh argv.
#[tauri::command]
async fn vpn_toggle(
    state: tauri::State<'_, std::sync::Arc<VpnState>>,
    path: String,
    on: bool,
) -> Result<(), String> {
    let handle = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let defs = vpn::load(&patchbay::config_path())?;
        let v = defs
            .get(&path)
            .ok_or_else(|| format!("no [vpn.\"{path}\"] in the config"))?;
        let r = v.resolve();
        let cmd = if on { r.up } else { r.down };
        let cmd = cmd.ok_or_else(|| {
            format!("[vpn.\"{path}\"] has no `{}` command", if on { "up" } else { "down" })
        })?;
        vpn::run(&cmd)?;
        handle.0.lock().unwrap().insert(path, on);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn save_vpn(path: String, def: vpn::Vpn) -> Result<(), String> {
    config::save_vpn(&path, &def)
}

#[derive(Serialize)]
struct TunnelView {
    id: u32,
    jack: String,
    local: u16,
    via: String,
}

/// Opens a device's remote desktop. If it sits behind a jump chain, forward a
/// local port over that chain first — RDP has no ProxyJump of its own.
#[tauri::command]
async fn open_rdp(
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    name: String,
) -> Result<String, String> {
    let shared = tunnels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let jacks = read()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let j = jacks.get(&resolved).ok_or("no such device")?;
        let port = j.rdp.ok_or_else(|| format!("\"{resolved}\" has no rdp port"))?;
        let hops = patchbay::hops(&resolved, &jacks)?;

        let addr = if hops.is_empty() {
            format!("{}:{port}", j.host)
        } else {
            let local = rdp::free_port()?;
            // -N: no shell, just the forward. Same -J chain the terminal would use.
            let mut args = vec![
                "-N".to_string(),
                "-L".to_string(),
                format!("127.0.0.1:{local}:{}:{port}", j.host),
                "-J".to_string(),
                hops.join(","),
            ];
            // The chain's far end is the box we tunnel from, not the target.
            args.push(hops.last().cloned().unwrap_or_default());
            // A clock-derived id can collide, and a collision would overwrite the
            // map entry and leak the child with nothing left to kill it.
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            shared.open(id, &resolved, &args, local, hops.join(" → "))?;
            format!("127.0.0.1:{local}")
        };

        let body = rdp::rdp_file(&addr, j.user.as_deref())?;
        let path = rdp::write_file(&resolved, &body)?;
        os_open(path.as_os_str())?;
        Ok(addr)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn tunnels(state: tauri::State<'_, rdp::SharedTunnels>) -> Vec<TunnelView> {
    state
        .list()
        .into_iter()
        .map(|(id, jack, local, via)| TunnelView { id, jack, local, via })
        .collect()
}

#[tauri::command]
fn close_tunnel(state: tauri::State<'_, rdp::SharedTunnels>, id: u32) {
    state.close(id);
}

#[tauri::command]
fn colors() -> std::collections::BTreeMap<String, String> {
    config::load_colors()
}

#[tauri::command]
fn save_color(os: String, hex: Option<String>) -> Result<(), String> {
    config::save_color(&os, hex.as_deref())
}

#[tauri::command]
fn settings() -> config::Settings {
    config::load_settings()
}

#[tauri::command]
fn save_settings(next: config::Settings) -> Result<(), String> {
    config::save_settings(&next)
}

/// Which VPN presets this machine can actually drive, and the profiles they know
/// about — so the sheet offers a pick list instead of asking you to type.
#[tauri::command]
async fn vpn_providers() -> Vec<vpn::Provider> {
    tauri::async_runtime::spawn_blocking(vpn::providers)
        .await
        .unwrap_or_default()
}

#[tauri::command]
fn delete_vpn(path: String) -> Result<(), String> {
    config::delete_vpn(&path)
}

/// The raw definition, for the edit sheet — `vpns` returns live state instead.
#[tauri::command]
fn vpn_def(path: String) -> Result<Option<vpn::Vpn>, String> {
    Ok(vpn::load(&patchbay::config_path())?.get(&path).cloned())
}

#[tauri::command]
fn config_path() -> String {
    patchbay::config_path().display().to_string()
}

#[tauri::command]
fn open_config() -> Result<(), String> {
    os_open(patchbay::config_path().as_os_str())
}

fn main() {
    tauri::Builder::default()
        .manage(pty::Shared::default())
        .manage(std::sync::Arc::<VpnState>::default())
        .manage(rdp::SharedTunnels::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let w = app.get_webview_window("main").unwrap();
                // Sidebar, not HudWindow: HUD material is built for floating panels
                // and is transparent enough that sidebar text washes out over a
                // bright desktop. This is the material Finder and Mail use.
                let _ = apply_vibrancy(&w, NSVisualEffectMaterial::Sidebar, None, Some(12.0));
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            jacks, connect, probe, config_path, open_config,
            save_jack, delete_jack, rename_group, delete_group, open_url,
            vpns, vpn_toggle, vpn_def, save_vpn, delete_vpn, vpn_providers,
            settings, save_settings, colors, save_color,
            open_rdp, tunnels, close_tunnel,
            open_session, write_session, resize_session, close_session
        ])
        .build(tauri::generate_context!())
        .expect("error while building patchbay")
        .run(|handle, event| {
            // `ssh -N -L` has no parent to hang up on, so without this a tunnel
            // outlives the window and keeps holding its forwarded port.
            if matches!(event, tauri::RunEvent::Exit) {
                handle.state::<rdp::SharedTunnels>().close_all();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::is_web_url;

    #[test]
    fn only_http_and_https_are_openable() {
        assert!(is_web_url("https://10.0.0.20:5001"));
        assert!(is_web_url("HTTP://nas.local/"));
        assert!(is_web_url("  https://nas.local  "));
        // the desktop opener would launch an app for these
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "x-apple-helpme://boom",
            "smb://share",
            "nas.local:5001",
            "",
            "https://ok\nfile:///etc/passwd",
        ] {
            assert!(!is_web_url(bad), "{bad:?} should be rejected");
        }
    }
}
