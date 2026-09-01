#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod config;
mod import;
mod patchbay;
mod pty;
mod rdp;
mod rdp_session;
mod team;
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
    folders: Vec<String>,
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
            folders: j.folders.clone().unwrap_or_default(),
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

/// `ping -c 5` / `traceroute`, the spelling the far side will have. Also the local
/// spelling everywhere except Windows.
fn posix_task(task: &str, host: &str) -> Vec<String> {
    match task {
        "trace" => vec!["traceroute".into(), host.into()],
        _ => vec!["ping".into(), "-c".into(), "5".into(), host.into()],
    }
}

#[cfg(not(target_os = "windows"))]
fn local_task(task: &str, host: &str) -> (String, Vec<String>) {
    let mut v = posix_task(task, host);
    (v.remove(0), v)
}

#[cfg(target_os = "windows")]
fn local_task(task: &str, host: &str) -> (String, Vec<String>) {
    match task {
        "trace" => ("tracert".into(), vec![host.into()]),
        _ => ("ping".into(), vec!["-n".into(), "5".into(), host.into()]),
    }
}

/// The far-side form hands the host to the hop's shell, so a space or a semicolon in
/// it would run there as a command — the same hole as a newline in a `.rdp`, closed
/// the same way: reject rather than quote. A leading `-` would be an option, locally
/// too.
fn plain_host(host: &str) -> Result<&str, String> {
    let ok = !host.is_empty()
        && !host.starts_with('-')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || ".:-_".contains(c));
    ok.then_some(host)
        .ok_or_else(|| format!("\"{host}\" isn't a plain host name to check"))
}

/// A one-shot check has no use for the hop's tunnels, and re-binding a port a live
/// session already holds only prints an error into the tab.
fn without_forwards(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "-L" {
            it.next();
        } else {
            out.push(a);
        }
    }
    out
}

/// Where a reachability check has to run to mean anything. A device behind a jump is
/// not reachable from here at all — pinging a name only its bastion can resolve
/// proves nothing — so the check runs *on the hop*, which is also the only machine
/// the status dot can probe toward.
fn task_argv(
    task: &str,
    name: &str,
    jacks: &patchbay::Jacks,
) -> Result<(String, Vec<String>), String> {
    if task != "ping" && task != "trace" {
        return Err(format!("no task named \"{task}\""));
    }
    let j = jacks
        .get(name)
        .ok_or_else(|| format!("no jack named \"{name}\""))?;
    let host = plain_host(&j.host)?;

    let Some(hop) = &j.jump else {
        return Ok(local_task(task, host));
    };
    // ponytail: the far side is assumed POSIX — it answers ssh, so it isn't cmd.exe.
    // A Windows bastion would need the local spelling pushed through instead.
    let mut args = match jacks.contains_key(hop) {
        true => without_forwards(patchbay::ssh_args(hop, jacks)?),
        // A jump that isn't a jack is a raw ssh spec, and has no chain of its own.
        false => vec![hop.clone()],
    };
    args.extend(posix_task(task, host));
    Ok(("ssh".into(), args))
}

/// Ping or traceroute in a tab. The same pty a session uses, so output streams and
/// ^C works; the tab goes dead when the command exits.
#[tauri::command]
fn open_task(
    app: tauri::AppHandle,
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    name: String,
    task: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let jacks = read()?;
    let resolved = patchbay::resolve(&name, &jacks)?;
    let (program, args) = task_argv(&task, &resolved, &jacks)?;
    sessions.open(&app, id, &program, &args, cols.max(2), rows.max(2))?;
    Ok(terminal::command_line_of(&program, &args))
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
    sessions.open(&app, id, "ssh", &args, cols.max(2), rows.max(2))?;
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

#[derive(Serialize)]
struct SshHosts {
    path: String,
    hosts: Vec<import::Imported>,
    warnings: Vec<String>,
}

/// What an ssh config could become. Nothing is written here — the window shows the
/// list and writes only what gets ticked, through `save_jack` like every other edit.
/// ponytail: `~/.ssh/config` only. `bay import <file>` takes a path for the odd
/// case, and a picker in the window would be a file dialog for a file that is always
/// in the same place.
#[tauri::command]
fn ssh_hosts() -> Result<SshHosts, String> {
    let path = dirs::home_dir()
        .ok_or("no home directory to find an ssh config in")?
        .join(".ssh")
        .join("config");
    let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let found = import::from_ssh_config(&src);
    Ok(SshHosts {
        path: path.display().to_string(),
        hosts: found.hosts,
        warnings: found.warnings,
    })
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

/// A jack's `url`, resolved and checked, for whichever of the two openers wants it.
fn web_url_of(name: &str) -> Result<(String, String), String> {
    let jacks = read()?;
    let resolved = patchbay::resolve(name, &jacks)?;
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
    Ok((resolved, url))
}

/// Opens a jack's web UI in the real browser. Still here, and still the fallback:
/// it is the only one of the two with a certificate interstitial.
#[tauri::command]
fn open_url(name: String) -> Result<String, String> {
    let (_, url) = web_url_of(&name)?;
    os_open(url.as_ref())?;
    Ok(url)
}

/// One request of our own before the url reaches a webview, because a webview has no
/// "proceed anyway" for a certificate this machine doesn't trust — it paints nothing
/// at all and looks like a broken app. This is a deliberate click, not a background
/// sweep of every host, which is the thing the no-favicons rule is actually about.
fn web_reachable(url: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("no http client: {e}"))?;
    client.get(url).send().map(|_| ()).map_err(|e| {
        // reqwest's own Display is "error sending request for url (…)" and stops
        // there — the reason is only ever in the source chain, so walk it. Without
        // this the dialog repeats the url back at you and says nothing.
        let mut why = e.to_string();
        let mut cause: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
        while let Some(c) = cause {
            why = format!("{why}: {c}");
            cause = c.source();
        }
        // rustls names it differently depending on what's wrong with the chain; they
        // all mean the same thing to the person looking at the dialog.
        if ["certificate", "UnknownIssuer", "NotValidForName", "CertExpired"]
            .iter()
            .any(|s| why.contains(s))
        {
            // Naming the way out, because otherwise this is a dead end that sends
            // you to the browser forever for a device you could trust once. Kept
            // platform-neutral: the store is Keychain here and something else there.
            format!(
                "\"{url}\" uses a certificate this machine doesn't trust, so a window \
                 here would show nothing — trust it on this machine and it opens in the app"
            )
        } else {
            format!("\"{url}\": {why}")
        }
    })
}

/// The web UI in a window of its own. Not a tab with an iframe: DSM, OPNsense and
/// Proxmox all send `X-Frame-Options`, so the one thing an iframe could show is the
/// devices nobody points a `url` at.
///
/// It gets no capability, and must not: a remote origin matches no `ExecutionContext`
/// in `capabilities/`, so the appliance's own page cannot reach a single one of our
/// commands. Granting `remote` there would hand every device's web UI `delete_jack`.
#[tauri::command]
async fn open_web_window(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let (resolved, url) = web_url_of(&name)?;
    // The url is part of the label, not just the jack name. Keyed on the name alone,
    // a window opened once was focused by every later click — skipping the preflight
    // and still showing the page it first loaded, so editing a jack's url appeared to
    // do nothing and a failed load stayed on screen for good.
    // Not a hash for secrecy, just something to tell two urls apart in a label, so
    // std's is the right one and it needs to be stable only for this process.
    let tag = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut h);
        h.finish()
    };
    let label = format!(
        "web-{}-{tag:x}",
        resolved.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>(),
    );
    // The same jack at the same url is a focus, not a second window and a build error.
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        return Ok(());
    }

    let checked = url.clone();
    tauri::async_runtime::spawn_blocking(move || web_reachable(&checked))
        .await
        .map_err(|e| e.to_string())??;

    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::External(url.parse().map_err(|e| format!("\"{url}\": {e}"))?),
    )
    .title(format!("{resolved} — {url}"))
    .inner_size(1200.0, 900.0)
    .build()
    .map_err(|e| format!("\"{resolved}\": {e}"))?;
    Ok(())
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
        let (addr, resolved, user) = rdp_address(&shared, &name)?;
        let body = rdp::rdp_file(&addr, user.as_deref())?;
        let path = rdp::write_file(&resolved, &body)?;
        os_open(path.as_os_str())?;
        Ok(addr)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where to dial a device for RDP, forwarding a local port over the jump chain when
/// it sits behind one. Shared by the handoff above and the in-app session below, so
/// both reach a bastioned host the same way.
fn rdp_address(
    shared: &rdp::SharedTunnels,
    name: &str,
) -> Result<(String, String, Option<String>), String> {
    let jacks = read()?;
    let resolved = patchbay::resolve(name, &jacks)?;
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
    Ok((addr, resolved, j.user.clone()))
}

/// `DOMAIN\user` is how Windows people write a domain login, but RDP wants the two
/// as separate fields. A UPN (`user@domain`) is already one field, so it passes through.
pub fn split_domain(user: &str) -> (Option<String>, String) {
    let Some((domain, name)) = user.split_once('\\') else {
        return (None, user.to_string());
    };
    // `.\alice` is how mstsc's box says "a local account", and it resolves that dot
    // itself rather than putting a domain of "." on the wire.
    ((!domain.is_empty() && domain != ".").then(|| domain.to_string()), name.to_string())
}

/// Remote desktop in a tab instead of the system client. Connects synchronously so a
/// wrong password is a returned error the sheet can show, then streams tiles.
#[tauri::command]
async fn open_rdp_session(
    app: tauri::AppHandle,
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    sessions: tauri::State<'_, rdp_session::Shared>,
    id: u32,
    name: String,
    user: String,
    password: String,
    width: u16,
    height: u16,
    on_tile: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<rdp_session::Screen, String> {
    let shared = tunnels.inner().clone();
    let rdp_sessions = sessions.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (addr, resolved, cfg_user) = rdp_address(&shared, &name)?;
        let (host, port) = addr
            .rsplit_once(':')
            .ok_or_else(|| format!("\"{addr}\" isn\'t a host and port"))?;
        let port: u16 = port.parse().map_err(|_| format!("\"{addr}\" has no usable port"))?;
        // The window asks for a sign-in, so a jack with no `user` still connects —
        // the config's user is only what the field is prefilled with.
        let user = Some(user)
            .filter(|u| !u.is_empty())
            .or(cfg_user)
            .ok_or_else(|| format!("\"{resolved}\" needs a user to sign in with"))?;
        let (domain, user) = split_domain(&user);
        rdp_sessions.open(id, host, port, user, password, domain, width, height, on_tile, app)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn close_rdp_session(sessions: tauri::State<'_, rdp_session::Shared>, id: u32) {
    sessions.close(id);
}

/// Mouse and keyboard from the canvas. Fire-and-forget: an input that arrives after
/// the session ended is not an error worth a dialog.
#[tauri::command]
fn rdp_input(
    sessions: tauri::State<'_, rdp_session::Shared>,
    id: u32,
    kind: String,
    a: i32,
    b: i32,
    down: bool,
) {
    let input = match kind.as_str() {
        "move" => rdp_session::Input::Move { x: a.clamp(0, 65535) as u16, y: b.clamp(0, 65535) as u16 },
        "button" => rdp_session::Input::Button { button: a.clamp(0, 255) as u8, down },
        "wheel" => rdp_session::Input::Wheel { delta: a.clamp(-32768, 32767) as i16 },
        "key" => rdp_session::Input::Key { scancode: a.clamp(0, 65535) as u16, down },
        _ => return,
    };
    sessions.send(id, input);
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

/// Private keys in `~/.ssh`, for the key field to suggest. A key is recognised by its
/// `.pub` sibling, which leaves out `config` and `known_hosts` without naming them.
///
/// They come back `~`-prefixed on purpose: a key path ends up in a config a colleague
/// may open, and their home directory isn't yours. Both front ends expand `~`.
#[tauri::command]
fn ssh_keys() -> Vec<String> {
    match dirs::home_dir() {
        Some(h) => keys_in(&h.join(".ssh")),
        None => vec![],
    }
}

fn keys_in(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut keys: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // The public half sits beside the private one, and that's the whole test.
            // `.pub` on the end rules out the public halves themselves.
            let paired = e.path().with_file_name(format!("{name}.pub")).is_file();
            (!name.ends_with(".pub") && paired).then(|| format!("~/.ssh/{name}"))
        })
        .collect();
    keys.sort();
    keys
}

#[tauri::command]
fn defaults() -> config::Defaults {
    config::load_defaults()
}

#[tauri::command]
fn save_defaults(next: config::Defaults) -> Result<(), String> {
    config::save_defaults(&next)
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

/// One call for the whole loop — fetch, then push or adopt, whichever applies. The
/// window runs it on focus and after every edit; with no team it returns immediately
/// and touches nothing.
#[tauri::command]
async fn team_sync() -> team::Status {
    tauri::async_runtime::spawn_blocking(team::sync)
        .await
        .unwrap_or_else(|e| team::Status {
            state: "offline",
            url: String::new(),
            code: String::new(),
            seats: 0,
            paid: false,
            error: Some(e.to_string()),
            changed: false,
        })
}

#[tauri::command]
async fn team_join(url: String, code: String) -> Result<team::Status, String> {
    tauri::async_runtime::spawn_blocking(move || team::join(&url, &code))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn team_create(url: String) -> Result<team::Status, String> {
    tauri::async_runtime::spawn_blocking(move || team::create(&url))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn team_resolve(keep: String) -> Result<team::Status, String> {
    tauri::async_runtime::spawn_blocking(move || team::resolve(&keep))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn team_leave() -> Result<(), String> {
    team::leave()
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
        .manage(rdp_session::Shared::default())
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
            settings, save_settings, colors, save_color, defaults, save_defaults, ssh_keys,
            ssh_hosts,
            team_sync, team_join, team_create, team_resolve, team_leave, open_web_window,
            open_rdp, open_rdp_session, close_rdp_session, rdp_input, tunnels, close_tunnel,
            open_session, open_task, write_session, resize_session, close_session
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
    use super::{is_web_url, keys_in, split_domain, task_argv, web_reachable};

    const CHAIN: &str = r#"
[jack.bastion]
host = "bastion.example"
user = "ops"
port = 2222
forward = ["9000:localhost:9000"]

[jack.db]
host = "db.internal"
jump = "bastion"

[jack.plain]
host = "10.0.0.4"

[jack.raw]
host = "10.0.0.9"
jump = "ops@edge.example"

[jack.sneaky]
host = "x; id"
"#;

    #[test]
    fn a_check_runs_here_when_it_can_and_on_the_hop_when_it_cannot() {
        let j = super::patchbay::parse(CHAIN).unwrap();

        // Nothing in the way: straight at the host.
        let (p, a) = task_argv("ping", "plain", &j).unwrap();
        assert_eq!(p, "ping");
        assert!(a.contains(&"10.0.0.4".to_string()), "got {a:?}");

        // Behind a bastion, so it runs there — and the hop's own tunnel is left out,
        // or it fights the live session for the port.
        let (p, a) = task_argv("ping", "db", &j).unwrap();
        assert_eq!(p, "ssh");
        assert_eq!(a, ["-p", "2222", "ops@bastion.example", "ping", "-c", "5", "db.internal"]);

        let (_, a) = task_argv("trace", "db", &j).unwrap();
        assert_eq!(a.last().unwrap(), "db.internal");
        assert!(a.contains(&"traceroute".to_string()), "got {a:?}");

        // A jump that isn't a jack is a raw spec with no chain of its own.
        let (_, a) = task_argv("ping", "raw", &j).unwrap();
        assert_eq!(a[0], "ops@edge.example");
    }

    #[test]
    fn a_host_that_could_be_a_command_on_the_hop_is_refused() {
        let j = super::patchbay::parse(CHAIN).unwrap();
        assert!(task_argv("ping", "sneaky", &j).is_err(), "a space reaches the hop's shell");
        assert!(task_argv("nope", "plain", &j).is_err(), "only the two tasks exist");
    }

    #[test]
    fn only_private_keys_with_a_public_half_are_suggested() {
        let dir = std::env::temp_dir().join(format!("patchbay-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["id_ed25519", "id_ed25519.pub", "known_hosts", "config", "work.key", "work.key.pub"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        assert_eq!(keys_in(&dir), ["~/.ssh/id_ed25519", "~/.ssh/work.key"]);
    }

    #[test]
    fn a_domain_login_splits_into_the_two_fields_rdp_wants() {
        assert_eq!(split_domain("CORP\\alice"), (Some("CORP".into()), "alice".into()));
        assert_eq!(split_domain("alice"), (None, "alice".into()));
        assert_eq!(split_domain("alice@corp.example"), (None, "alice@corp.example".into()));
        // Entra-joined boxes want the UPN kept whole behind the AzureAD prefix.
        assert_eq!(
            split_domain("AzureAD\\alice@corp.example"),
            (Some("AzureAD".into()), "alice@corp.example".into())
        );
        assert_eq!(split_domain(".\\alice"), (None, "alice".into()), "a local account");
        assert_eq!(split_domain("\\alice"), (None, "alice".into()));
    }

    /// reqwest's own Display is "error sending request for url (…)" and stops there,
    /// so the dialog repeated the url back at the user and said nothing about why.
    /// The reason only ever lives in the source chain.
    #[test]
    fn a_web_ui_that_wont_load_says_why_not_just_which() {
        let err = web_reachable("https://127.0.0.1:1").unwrap_err();
        assert!(err.contains("127.0.0.1:1"), "{err}");
        assert!(
            err.to_lowercase().contains("refused"),
            "the cause chain wasn't walked, so the message names no cause: {err}"
        );
    }

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
