#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod patchbay;
mod pty;
mod terminal;

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
    if !path.exists() {
        return Err(format!(
            "no config at {} — run `bay edit` to start one",
            path.display()
        ));
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

#[tauri::command]
fn config_path() -> String {
    patchbay::config_path().display().to_string()
}

#[tauri::command]
fn open_config() -> Result<(), String> {
    let path = patchbay::config_path();
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(&path);
        c
    } else if cfg!(target_os = "windows") {
        // `explorer <file>` exits non-zero even when it worked; `start` is the
        // form that actually reports success. The empty "" is start's title arg.
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", ""]).arg(&path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&path);
        c
    };
    cmd.spawn()
        .map_err(|e| format!("could not open {}: {e}", path.display()))?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(pty::Shared::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let w = app.get_webview_window("main").unwrap();
                // The thing that makes it read as a Mac app instead of a web page.
                let _ = apply_vibrancy(&w, NSVisualEffectMaterial::HudWindow, None, Some(12.0));
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            jacks, connect, probe, config_path, open_config,
            save_jack, delete_jack, rename_group, delete_group,
            open_session, write_session, resize_session, close_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running patchbay");
}
