#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod config;
mod import;
mod patchbay;
mod pty;
mod rdp;
mod rdp_session;
mod sftp;
mod terminal;

use serde::Serialize;
use std::path::{Path, PathBuf};
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
    vnc: Option<u16>,
    ssh: bool,
    primary: String,
    desc: Option<String>,
    folders: Vec<String>,
    forward: Vec<String>,
    /// Ordered hops, first one nearest us - what the detail pane draws as the route.
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



/// No config at all is not an error in the window - it's a first run, and the UI has
/// somewhere to put that. A config that exists but won't parse still is one.
fn read() -> Result<patchbay::Jacks, String> {
    patchbay::load(&patchbay::config_path())
}

/// Where ssh keeps its own config, and where ours goes beside it.
fn ssh_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh"))
}

/// Keep `~/.ssh/patchbay.conf` current, if it was asked for.
///
/// Called from `jacks` rather than from each of the eight writers: this is the one
/// place that has already read the list, and a hand-edit changes it without any
/// writer here running at all, and a generator that has to be remembered at eight call
/// sites is a generator that goes stale. Nothing is written when the text hasn't
/// changed, so a window focus costs a read and a compare.
///
/// Quiet on failure on purpose - a home directory we can't write to is not a reason to
/// fail the device list, which is what the window actually asked for.
fn sync_ssh_config(jacks: &patchbay::Jacks) {
    if !config::load_settings().write_ssh_config {
        return;
    }
    let Some(dir) = ssh_dir() else { return };
    let theirs = std::fs::read_to_string(dir.join("config"))
        .map(|s| import::host_names(&s))
        .unwrap_or_default();
    let _ = config::write_ssh_include(&dir, &import::to_ssh_config(jacks, &theirs));
}

#[tauri::command]
fn jacks() -> Result<Vec<JackView>, String> {
    let jacks = read()?;
    sync_ssh_config(&jacks);
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
            vnc: j.vnc,
            ssh: j.ssh.unwrap_or(true),
            primary: patchbay::primary(j),
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
/// opened and dropped. ponytail: one thread per jack, fine to a few hundred - swap for
/// a bounded pool if someone shows up with a thousand.
#[tauri::command]
async fn probe() -> Result<Vec<Probe>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let jacks = read()?;
        let targets: Vec<(String, String, u16)> = jacks
            .keys()
            .filter_map(|n| patchbay::entry(n, &jacks).ok().map(|(h, p)| (n.clone(), h, p)))
            .collect();

        // ponytail: one thread per jack, bounded pool if someone brings a thousand
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
/// it would run there as a command - the same hole as a newline in a `.rdp`, closed
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
        if patchbay::FORWARD_FLAGS.contains(&a.as_str()) {
            it.next();
        } else {
            out.push(a);
        }
    }
    out
}

/// Where a reachability check has to run to mean anything. A device behind a jump is
/// not reachable from here at all - pinging a name only its bastion can resolve
/// proves nothing - so the check runs *on the hop*, which is also the only machine
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
    // ponytail: the far side is assumed POSIX - it answers ssh, so it isn't cmd.exe.
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
    sessions.open(&app, id, &program, &args, cols.max(2), rows.max(2), None)?;
    Ok(terminal::command_line_of(&program, &args))
}

/// A device name as a filename: whatever survives isn't a path, so a hand-edited
/// config with a slash or a dot in a jack name can't land the log somewhere else.
fn log_path(name: &str) -> std::path::PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    patchbay::logs_dir().join(format!("{safe}-{stamp}.log"))
}

/// Opens a session in the window. `connect` is still there for "open in my real
/// terminal" - this is the in-app one.
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
    // The shell is also the connection the file browser rides: `sftp -b` cannot ask
    // for a password, so a session here is what authenticates it. Not shown in the
    // command line below - that is the command, not our plumbing.
    let mux = sftp::mux(&sftp::control_path(&resolved));
    let spawned: Vec<String> = mux.into_iter().chain(args.iter().cloned()).collect();
    let log = config::load_settings().log_sessions.then(|| log_path(&resolved));
    sessions.open(&app, id, "ssh", &spawned, cols.max(2), rows.max(2), log)?;
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
    config::save_jack_at(&patchbay::config_path(), original, jack)
}

#[derive(Serialize)]
struct SshHosts {
    path: String,
    hosts: Vec<import::Imported>,
    warnings: Vec<String>,
}

/// What an ssh config could become. Nothing is written here - the window shows the
/// list and writes only what gets ticked, through `save_jack` like every other edit.
/// `~/.ssh/config` only: a picker would be a file dialog for a file that is always
/// in the same place.
/// A Royal TS document, read in the window and handed here as text - so there is no
/// file dialog, no plugin and no new capability, and the app never opens a path it
/// wasn't given. Parses only, like the ssh side: what gets written is what you tick.
#[tauri::command]
fn royal_hosts(src: String) -> Result<SshHosts, String> {
    let found = import::from_royal_ts(&src)?;
    Ok(SshHosts { path: String::new(), hosts: found.hosts, warnings: found.warnings })
}

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
    config::delete_jack_at(&patchbay::config_path(), &name)
}

/// The notes hung on folders, by folder path.
#[tauri::command]
fn notes() -> Vec<Note> {
    let src = std::fs::read_to_string(patchbay::config_path()).unwrap_or_default();
    patchbay::notes(&src)
        .into_iter()
        .map(|(path, note)| Note { path, note })
        .collect()
}

#[derive(serde::Serialize)]
struct Note {
    path: String,
    note: String,
}

#[tauri::command]
fn save_note(path: String, note: String) -> Result<(), String> {
    config::set_note_at(&patchbay::config_path(), &path, &note)
}

#[tauri::command]
fn rename_group(from: String, to: String) -> Result<usize, String> {
    config::rename_group_at(&patchbay::config_path(), &from, &to)
}

#[tauri::command]
fn delete_group(path: String) -> Result<usize, String> {
    config::delete_group_at(&patchbay::config_path(), &path)
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

/// A link in terminal output: text a remote host wrote, so it goes nowhere until
/// someone clicks it, and nowhere but a browser. `is_web_url` is the same http(s)
/// check a jack's `url` passes, and `os_open` never goes through a shell.
#[tauri::command]
fn open_link(url: String) -> Result<String, String> {
    if !is_web_url(&url) {
        return Err(format!("not a web address: \"{url}\""));
    }
    os_open(url.as_ref())?;
    Ok(url)
}

/// One request of our own before the url reaches a webview, because a webview has no
/// "proceed anyway" for a certificate this machine doesn't trust - it paints nothing
/// at all and looks like a broken app. This is a deliberate click, not a background
/// sweep of every host, which is the thing the no-favicons rule is actually about.
fn web_reachable(url: &str) -> Result<(), String> {
    // A NAS asleep on its own hibernation timer drops the first SYN and takes its
    // time coming back - 8s wasn't enough for a DS920+ waking up. A device that is
    // simply off costs the full wait, but "it's not answering" arriving late beats
    // it arriving wrong about a device that was only asleep.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("no http client: {e}"))?;
    client.get(url).send().map(|_| ()).map_err(|e| {
        // reqwest's own Display is "error sending request for url (…)" and stops
        // there - the reason is only ever in the source chain, so walk it. Without
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
                 here would show nothing - trust it on this machine and it opens in the app"
            )
        } else {
            format!("\"{url}\": {why}")
        }
    })
}

/// A device's web UI as a **tab**, the way pty.rs and rdp_session.rs are tabs - a
/// child webview inside the main window, not an iframe and not a window of its own.
/// An iframe is what `X-Frame-Options` blocks, and DSM, OPNsense and Proxmox all send
/// it; a separate window isn't where the rest of the app's sessions live.
///
/// The webview is an OS-level view stacked *above* the page, so it obeys none of our
/// CSS. The window tells us where to put it and shrinks it to nothing to get it out of
/// the way - see `place_web_view`. Every overlay has to do that or it paints over them.
///
/// It gets no capability, and must not: a remote origin matches no `ExecutionContext`
/// in `capabilities/`, so the appliance's own page cannot reach a single one of our
/// commands. Granting `remote` there would hand every device's web UI `delete_jack`.
#[tauri::command]
async fn open_web_view(
    app: tauri::AppHandle,
    id: u32,
    name: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    let (resolved, url) = web_url_of(&name)?;
    let parsed: tauri::Url = url.parse().map_err(|e| format!("\"{url}\": {e}"))?;
    if parsed.scheme() == "http" && !parsed.host_str().is_some_and(is_private_host) {
        // Say it and mean it: the browser is where a cleartext page on the public
        // internet goes, and it has its own opinions to show about that.
        os_open(url.as_ref())?;
        return Err(format!(
            "\"{url}\" is plain http to a public address - patchbay opens cleartext \
             only on your own network, so it opened in your browser instead"
        ));
    }
    let window = app.get_window("main").ok_or("the main window has gone")?;

    // Deliberately no reachability check on this path. It cost a whole round trip
    // before anything appeared, which is most of "the websites load a while" - the
    // view goes up now and `web_check` reports a bad certificate alongside it.
    // Where it ends up is not where it was sent. A Synology's http port is a three-line
    // script that redirects to its https one, so the certificate that silently blanks
    // the page belongs to a url we were never given - and a JS redirect is invisible to
    // an http client. This is the only way to learn the real destination.
    let reporter = app.clone();
    window
        .add_child(
            tauri::webview::WebviewBuilder::new(web_label(id), tauri::WebviewUrl::External(parsed))
                .on_navigation(move |to| {
                    use tauri::Emitter;
                    let _ = reporter.emit(&format!("web-nav:{id}"), to.to_string());
                    true
                }),
            tauri::LogicalPosition::new(x, y),
            tauri::LogicalSize::new(width.max(1.0), height.max(1.0)),
        )
        .map_err(|e| format!("\"{resolved}\": {e}"))?;
    Ok(url)
}

fn web_label(id: u32) -> String {
    format!("webtab-{id}")
}

/// Move and size a web tab, in logical pixels relative to the window. A zero size is
/// how it gets hidden: a child webview has no `hidden`, and it sits above every sheet
/// and the palette, so anything that opens over it has to call this first.
#[tauri::command]
fn place_web_view(
    app: tauri::AppHandle,
    id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let Some(w) = app.get_webview(&web_label(id)) else {
        // A place for a tab that has already gone is the ordinary result of a race
        // between closing one and a resize, not something to put in front of anyone.
        return Ok(());
    };
    w.set_position(tauri::LogicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    w.set_size(tauri::LogicalSize::new(width.max(0.0), height.max(0.0)))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn close_web_view(app: tauri::AppHandle, id: u32) {
    if let Some(w) = app.get_webview(&web_label(id)) {
        let _ = w.close();
    }
}

/// The old preflight, off the critical path: the tab is already up, so this only has
/// to say *why* a blank one is blank. Takes a url rather than a jack, because the one
/// worth checking is often the redirect's destination rather than the configured one.
#[tauri::command]
async fn web_check(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err("only http:// and https:// urls can be opened".into());
    }
    if web_trusted(&url) {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || web_reachable(&url))
        .await
        .map_err(|e| e.to_string())?
}

/// Beside the config, for the same reason `rdp_known_hosts` is.
fn web_trust_store() -> PathBuf {
    patchbay::config_path().with_file_name("web_trusted")
}

fn web_trusted(url: &str) -> bool {
    web_trusted_at(&web_trust_store(), url)
}

fn web_trusted_at(store: &Path, url: &str) -> bool {
    std::fs::read_to_string(store)
        .unwrap_or_default()
        .lines()
        .any(|l| l.trim() == url)
}

/// "Show it anyway", remembered. A NAS is reached by its IP, so its certificate names
/// something else and never will match - telling someone to re-address every device is
/// not a fix. We still can't make the webview accept it, because that challenge belongs
/// to wry; what this buys is that once the certificate *is* trusted on this machine,
/// our own stricter check stops hiding a page the webview will now render perfectly.
///
/// Deliberately not in the config: it is this machine's judgement about one device, and
/// the config is a document people hand-edit.
#[tauri::command]
fn web_trust(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err("only http:// and https:// urls can be opened".into());
    }
    web_trust_at(&web_trust_store(), &url)
}

/// Cleartext is for the LAN and nowhere else. The `Info.plist` exemption that lets a
/// webview load http at all is `NSAllowsArbitraryLoadsInWebContent`, which is broader
/// than we need - Apple offers nothing narrower that covers a bare `192.168.x.x`. So
/// the narrowing happens here instead: patchbay itself will only open cleartext to an
/// address that cannot be on the public internet, and a public http url goes to the
/// browser, which has its own opinions about that.
fn is_private_host(host: &str) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            // 100.64/10 is carrier-grade NAT, and it is what Tailscale hands every
            // node - the homelab this feature exists for is as likely to be reached
            // there as on 192.168. `is_private` is RFC 1918 only, and `is_shared`,
            // which would say this in one call, is still unstable.
            let o = v4.octets();
            let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
            v4.is_private()
                || cgnat
                || v4.is_loopback()
                || v4.is_link_local()
                || v4 == Ipv4Addr::UNSPECIFIED
        }
        Ok(IpAddr::V6(v6)) => {
            // Unique-local (fc00::/7) and link-local (fe80::/10), plus ::1.
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        // Not an address: `.local` is mDNS, and a name with no dot at all can only be
        // resolved by something on this network.
        Err(_) => {
            let h = host.to_ascii_lowercase();
            let h = h.strip_suffix('.').unwrap_or(&h);
            h == "localhost" || h.ends_with(".local") || h.ends_with(".home.arpa") || !h.contains('.')
        }
    }
}

/// A device's leaf certificate, fetched without judging it - macOS does the judging,
/// and it can't judge what it hasn't been shown. Same accept-anything verifier the RDP
/// side needs, for the same reason: we are looking at the certificate, not trusting it.
fn peer_cert(url: &str) -> Result<(String, Vec<u8>), String> {
    use std::io::Write as _;
    use std::net::ToSocketAddrs as _;
    use tokio_rustls::rustls;

    let u = tauri::Url::parse(url).map_err(|e| format!("\"{url}\": {e}"))?;
    let host = u.host_str().ok_or_else(|| format!("\"{url}\" has no host"))?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);

    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| format!("{host}:{port}: no address for that host"))?;
    let stream = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(10))
        .map_err(|e| format!("{host}:{port}: {e}"))?;

    let config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(rdp_session::verifier::AcceptAny))
        .with_no_client_auth();
    let name = host
        .clone()
        .try_into()
        .map_err(|_| format!("\"{host}\" isn't a usable server name"))?;
    let client = rustls::ClientConnection::new(std::sync::Arc::new(config), name)
        .map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(client, stream);
    // Without a flush the handshake hasn't moved far enough for a peer certificate.
    tls.flush().map_err(|e| format!("{host}: {e}"))?;

    let der = tls
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .map(|c| c.as_ref().to_vec())
        .ok_or_else(|| format!("{host} sent no certificate"))?;
    Ok((host, der))
}

/// What the user is agreeing to trust, in the words the OS dialog would use. Nobody
/// should be asked to trust a certificate they have not been shown - Safari puts the
/// subject, issuer and expiry in front of you, and until this we asked for the same
/// decision with only an error message on screen.
#[derive(Serialize)]
struct CertFacts {
    subject: String,
    issuer: String,
    expires: String,
    /// SHA-256 over the DER, the digest every other tool prints for a certificate.
    fingerprint: String,
}

fn cert_facts(der: &[u8]) -> Result<CertFacts, String> {
    use x509_cert::der::Decode as _;
    let c = x509_cert::Certificate::from_der(der).map_err(|e| e.to_string())?;
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    sha2::Digest::update(&mut h, der);
    let fingerprint = sha2::Digest::finalize(h)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    Ok(CertFacts {
        subject: c.tbs_certificate.subject.to_string(),
        issuer: c.tbs_certificate.issuer.to_string(),
        expires: c.tbs_certificate.validity.not_after.to_string(),
        fingerprint,
    })
}

/// Fetch and describe, without trusting anything. Split from `web_trust_cert` so the
/// window can show the certificate and *then* ask - one round trip each, rather than
/// one call that both reveals and commits.
#[tauri::command]
async fn web_cert(url: String) -> Result<serde_json::Value, String> {
    if !is_web_url(&url) {
        return Err("only http:// and https:// urls can be opened".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let (_, der) = peer_cert(&url)?;
        serde_json::to_value(cert_facts(&der)?).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Hand the certificate to macOS the way the browser's "Always trust" does. `security`
/// is the system's own tool and it raises the system's own authorisation prompt, so
/// nothing is trusted without the user's password - we never write trust settings
/// ourselves, we ask macOS to.
///
/// `-e hostnameMismatch` is the part that matters for an appliance: reached by its IP,
/// its certificate names something else, and that is the error being forgiven rather
/// than the issuer.
#[cfg(target_os = "macos")]
fn trust_cert(host: &str, der: &[u8]) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("patchbay-cert-{}.der", std::process::id()));
    std::fs::write(&path, der).map_err(|e| format!("{}: {e}", path.display()))?;
    let keychain = dirs::home_dir()
        .ok_or("no home directory")?
        .join("Library/Keychains/login.keychain-db");

    // `-s <host>` is load-bearing: it scopes the trust to connections to *this* device,
    // which is what Apple's own checkbox says and what we told the user. Without it,
    // `trustAsRoot -p ssl` makes the certificate a trusted SSL root outright - harmless
    // for a leaf that can only vouch for itself, and a very bad day if an appliance
    // hands us a CA certificate instead.
    let out = std::process::Command::new("/usr/bin/security")
        .args(["add-trusted-cert", "-r", "trustAsRoot", "-p", "ssl", "-e", "hostnameMismatch"])
        .args(["-s", host, "-k"])
        .arg(&keychain)
        .arg(&path)
        .output()
        .map_err(|e| format!("could not run security: {e}"));
    let _ = std::fs::remove_file(&path);
    let out = out?;

    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(match why.is_empty() {
        // Cancelling the system prompt is a decision, not a failure to report as one.
        true => "the certificate wasn't trusted".to_string(),
        false => format!("the certificate wasn't trusted: {why}"),
    })
}

/// Trust a device's certificate on this machine, so the webview will load its page.
/// Only macOS for now: elsewhere the webview reads a different store and this would
/// need its own spelling.
#[tauri::command]
async fn web_trust_cert(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err("only http:// and https:// urls can be opened".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let (host, der) = peer_cert(&url)?;
        trust_cert(&host, &der)?;
        // Our own check still refuses the name - rustls judges that itself and no trust
        // setting changes it - so record the override too, or the panel comes straight
        // back for a page that now loads.
        web_trust_at(&web_trust_store(), &url)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Windows keeps its own store and WebView2 reads it, so `certutil` is the local
/// spelling of the same idea. `-user` keeps it to this account: the machine store
/// needs an administrator and this is one person's decision about one appliance.
///
/// Narrower than the macOS path in one way worth knowing: Windows has no per-host
/// "allow this name mismatch". A certificate whose name doesn't match the address is
/// trusted as an issuer here and WebView2 may still refuse it, which is why the error
/// says so rather than reporting a success that doesn't hold.
#[cfg(target_os = "windows")]
fn trust_cert(_host: &str, der: &[u8]) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("patchbay-cert-{}.cer", std::process::id()));
    std::fs::write(&path, der).map_err(|e| format!("{}: {e}", path.display()))?;
    let out = std::process::Command::new("certutil")
        .args(["-user", "-addstore", "Root"])
        .arg(&path)
        .output()
        .map_err(|e| format!("could not run certutil: {e}"));
    let _ = std::fs::remove_file(&path);
    let out = out?;
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(match why.is_empty() {
        true => "the certificate wasn't trusted".to_string(),
        false => format!("the certificate wasn't trusted: {why}"),
    })
}

/// Linux has no one store - the webview reads the system bundle, and writing to it is
/// the distribution's business, not an app's.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn trust_cert(_host: &str, _der: &[u8]) -> Result<(), String> {
    Err("trusting a certificate from here isn't supported on this system - accept it in your browser instead".into())
}

fn web_trust_at(store: &Path, url: &str) -> Result<(), String> {
    if web_trusted_at(store, url) {
        return Ok(());
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store)
        .map_err(|e| format!("{}: {e}", store.display()))?;
    writeln!(f, "{url}").map_err(|e| format!("{}: {e}", store.display()))
}

#[derive(Serialize)]
struct TunnelView {
    id: u32,
    jack: String,
    local: u16,
    via: String,
}

/// Opens a device's remote desktop. If it sits behind a jump chain, forward a
/// local port over that chain first - RDP has no ProxyJump of its own.
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

/// Where to dial a device for RDP. Shared by the handoff above and the in-app session
/// below, so both reach a bastioned host the same way.
fn rdp_address(
    shared: &rdp::SharedTunnels,
    name: &str,
) -> Result<(String, String, Option<String>), String> {
    let jacks = read()?;
    let resolved = patchbay::resolve(name, &jacks)?;
    let j = jacks.get(&resolved).ok_or("no such device")?;
    let port = j.rdp.ok_or_else(|| format!("\"{resolved}\" has no rdp port"))?;
    let addr = dial_address(shared, &jacks, &resolved, port)?;
    Ok((addr, resolved, j.user.clone()))
}

/// A local address for a port ssh won't carry for us, forwarding over the jump chain
/// when the device sits behind one. RDP and VNC both need exactly this - it is what
/// Royal TS sells separately as Royal Server.
fn dial_address(
    shared: &rdp::SharedTunnels,
    jacks: &patchbay::Jacks,
    resolved: &str,
    port: u16,
) -> Result<String, String> {
    let j = jacks.get(resolved).ok_or("no such device")?;
    let hops = patchbay::hops(resolved, jacks)?;

    Ok(if hops.is_empty() {
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
        shared.open(next_tunnel_id(), resolved, &args, local, hops.join(" → "))?;
        format!("127.0.0.1:{local}")
    })
}

/// A clock-derived id can collide, and a collision would overwrite the map entry and
/// leak the child with nothing left to kill it.
fn next_tunnel_id() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The device's own `forward` list, held open on its own. The same `ssh -L` a session
/// would carry, minus the shell: a database port shouldn't come down because you
/// closed the terminal tab that happened to be holding it.
#[tauri::command]
async fn open_forwards(
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    name: String,
) -> Result<u16, String> {
    let shared = tunnels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let jacks = read()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let j = jacks
            .get(&resolved)
            .ok_or_else(|| format!("no jack named \"{resolved}\""))?;
        let forwards = j.forward.as_deref().unwrap_or_default();
        if forwards.is_empty() {
            return Err(format!("\"{resolved}\" has no forward to open"));
        }
        let ports: Vec<u16> = forwards.iter().filter_map(|f| patchbay::forward_local(f)).collect();
        // Every one of them, not just the one we watch: ExitOnForwardFailure ends ssh
        // if any single -L can't bind, so a clash on the second forward would surface
        // as the first one's twelve-second "never came up" and blame the network.
        if let Some(p) = ports.iter().copied().find(|p| rdp::port_taken(*p)) {
            return Err(format!("something is already listening on 127.0.0.1:{p}"));
        }
        // A `-R` binds on the far end, so a device whose forwards are all remote has
        // nothing here to connect to - `open` takes 0 and judges the tunnel by ssh
        // still being alive instead.
        let local = ports.first().copied().unwrap_or(0);
        // -N: no shell, just the forwards. ExitOnForwardFailure so a forward that
        // can't bind ends the tunnel instead of leaving a live ssh carrying nothing.
        let mut args = vec!["-N".to_string(), "-o".to_string(), "ExitOnForwardFailure=yes".to_string()];
        args.extend(patchbay::ssh_args(&resolved, &jacks)?);
        let hops = patchbay::hops(&resolved, &jacks)?;
        let via = if hops.is_empty() { patchbay::spec(j) } else { hops.join(" → ") };
        shared.open(next_tunnel_id(), &resolved, &args, local, via)?;
        Ok(local)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Screen sharing the way remote desktop is handed off: we never speak VNC, the OS
/// opens `vnc://` with whatever viewer is registered - Screen Sharing on macOS, and
/// on Windows or Linux whichever client claimed the scheme when it was installed.
#[tauri::command]
async fn open_vnc(
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    name: String,
) -> Result<String, String> {
    let shared = tunnels.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let jacks = read()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let j = jacks.get(&resolved).ok_or("no such device")?;
        let port = j.vnc.ok_or_else(|| format!("\"{resolved}\" has no vnc port"))?;
        let url = vnc_url(&dial_address(&shared, &jacks, &resolved, port)?, j.user.as_deref())?;
        os_open(std::ffi::OsStr::new(&url))?;
        Ok(url)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// This goes to the desktop opener, so it is checked on the way out as well as in: an
/// `@` or a `/` in the username would move the host the viewer dials. A name that
/// can't be carried safely is left out rather than mangled - the viewer asks for it.
fn vnc_url(addr: &str, user: Option<&str>) -> Result<String, String> {
    if !rdp::is_safe(addr) || addr.contains(['/', '@', '?', '#', ' ']) {
        return Err(format!("\"{addr}\" isn't a usable address"));
    }
    let ok = |u: &&str| u.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    Ok(match user.map(str::trim).filter(|u| !u.is_empty()).filter(ok) {
        Some(u) => format!("vnc://{u}@{addr}"),
        None => format!("vnc://{addr}"),
    })
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
        // The window asks for a sign-in, so a jack with no `user` still connects -
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

/// Pins the window and answers with what it is actually wearing. Two reasons the page
/// can't work this out for itself: macOS vibrancy and the title bar follow the
/// *window's* appearance rather than our CSS, and our own `color-scheme` makes
/// `prefers-color-scheme` echo back whatever we last pinned - so "system" asked in the
/// window stays on the last answer for ever.
#[tauri::command]
fn set_theme(window: tauri::WebviewWindow, theme: String) -> String {
    let want = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    let _ = window.set_theme(want);
    match want.or_else(|| window.theme().ok()) {
        Some(tauri::Theme::Light) => "light".into(),
        _ => "dark".into(),
    }
}

/// What a previous install left behind in `~/.ssh` after uninstall - the app can't
/// clean up when it isn't there to be asked, so it does it on first launch instead.
/// Only returned when the setting is currently off: if you're using it, the file
/// isn't a leftover, it's the feature working.
#[tauri::command]
fn ssh_leftovers() -> Option<config::Leftovers> {
    if config::load_settings().write_ssh_config {
        return None;
    }
    let dir = ssh_dir()?;
    let l = config::ssh_leftovers(&dir);
    (l.conf_file || l.include_line).then_some(l)
}

/// The click. Removes the file and takes the `Include` line back out of `~/.ssh/config`,
/// same code path as unticking the setting.
#[tauri::command]
fn clean_ssh_leftovers() -> Result<(), String> {
    let dir = ssh_dir().ok_or("no ~/.ssh on this machine")?;
    config::remove_ssh_include(&dir)
}

#[tauri::command]
fn save_settings(next: config::Settings) -> Result<(), String> {
    // Turning it off has to undo it here: `jacks` only ever writes the file, so with
    // the switch off nothing would ever come back and take it away again.
    let was = config::load_settings().write_ssh_config;
    config::save_settings(&next)?;
    if was && !next.write_ssh_config {
        if let Some(dir) = ssh_dir() {
            config::remove_ssh_include(&dir)?;
        }
    }
    Ok(())
}

/// Files over the existing connection. Each call is its own `sftp` run, sharing one
/// ssh session through multiplexing - see `sftp.rs`.
#[tauri::command]
async fn sftp_ls(name: String, path: String) -> Result<sftp::Listing, String> {
    tauri::async_runtime::spawn_blocking(move || sftp::ls(&name, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// Returns where it landed, so the window can say so rather than claiming success and
/// leaving the user to guess which folder.
#[tauri::command]
async fn sftp_get(name: String, remote: String, recurse: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        sftp::get(&name, &remote, &sftp::downloads(), recurse).map(|p| p.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The connection a files tab rides when nothing else has authenticated one yet:
/// `ssh -N` on a real pty, so a password, a host-key question or a key passphrase can
/// be answered in the tab itself rather than in a shell opened somewhere else. It runs
/// no command - its whole job is to be the master the `sftp` calls share, which is why
/// there is nothing to offer on Windows, where ssh has no multiplexing.
#[tauri::command]
fn open_master(
    app: tauri::AppHandle,
    sessions: tauri::State<'_, pty::Shared>,
    id: u32,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = (app, sessions, id, name, cols, rows);
        return Err("ssh on Windows can't share a connection, so files need a key or your agent".into());
    }
    #[cfg(not(windows))]
    {
        let jacks = read()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let args = without_forwards(patchbay::ssh_args(&resolved, &jacks)?);
        // -N first: everything after the destination would be a remote command.
        let mut spawned = vec!["-N".to_string()];
        spawned.extend(sftp::mux(&sftp::control_path(&resolved)));
        spawned.extend(args);
        sessions.open(&app, id, "ssh", &spawned, cols.max(2), rows.max(2), None)
    }
}

/// The Full Disk Access panel, for the case the file browser can name but not fix:
/// macOS hands `sftp-server` an empty Desktop, Documents or Downloads until it is
/// listed there. A fixed url, so nothing from a config reaches the opener.
#[tauri::command]
fn open_full_disk_access() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return os_open(std::ffi::OsStr::new(
        "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
    ));
    #[cfg(not(target_os = "macos"))]
    Err("that setting is a macOS one".into())
}

/// Polled by a files tab waiting on a shell to authenticate. A `stat`, not a
/// connection - asking by trying would be a failed login attempt every second.
#[tauri::command]
fn sftp_ready(name: String) -> Result<bool, String> {
    sftp::ready(&name)
}

#[tauri::command]
async fn sftp_put(name: String, local: String, remote_dir: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        sftp::put(&name, std::path::Path::new(&local), &remote_dir)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Making, renaming and removing. The other half of a file list: uploading into a
/// folder you have no way to create is half a file browser.
#[tauri::command]
async fn sftp_edit(name: String, op: String, path: String, to: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || sftp::edit(&name, &op, &path, &to))
        .await
        .map_err(|e| e.to_string())?
}

/// What a remote file is saved back by. There is no "open" over sftp and the far end
/// knows nothing about a file being edited, so this is the only signal there is.
#[derive(Clone, Serialize)]
struct SavedBack {
    name: String,
    file: String,
    error: Option<String>,
}

/// Open a remote file in whatever this machine opens it with, and put it back each time
/// it is saved: a download, the desktop opener, and a watch on the copy's mtime.
///
/// Returns where the copy is, so the window can say which file it handed over.
#[tauri::command]
async fn sftp_open(app: tauri::AppHandle, name: String, remote: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Where it goes back to. Both halves are checked where they are used - `get`
        // and `put` refuse a path sftp's batch language can't be given safely.
        let dir = remote.rsplit_once('/').map_or(".", |(d, _)| d).to_string();
        let local = sftp::get(&name, &remote, &sftp::edit_dir(&name), false)?;
        let seen = modified(&local);
        os_open(local.as_os_str())?;
        let at = local.display().to_string();
        // ponytail: one thread per file opened, polling. It ends when the copy is gone
        // or an upload is refused, and with the window otherwise; a platform watcher is
        // the upgrade if anyone opens dozens.
        std::thread::spawn(move || watch_edit(&app, &name, &local, &dir, seen));
        Ok(at)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// The other half of `sftp_open`, on its own thread. An editor writing the file is the
/// only thing that says a save happened, so the mtime is what is watched - and a
/// failed upload stops the watch rather than retrying at a host that just refused it.
fn watch_edit(
    app: &tauri::AppHandle,
    name: &str,
    local: &Path,
    dir: &str,
    mut seen: Option<std::time::SystemTime>,
) {
    use tauri::Emitter;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        // Gone: moved, or cleaned up. There is nothing left to save back.
        let Some(now) = modified(local) else { return };
        if Some(now) == seen {
            continue;
        }
        seen = Some(now);
        let error = sftp::put(name, local, dir).err();
        let failed = error.is_some();
        let _ = app.emit("sftp:saved", SavedBack {
            name: name.to_string(),
            file: local.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            error,
        });
        if failed {
            return;
        }
    }
}

/// A `patchbay://` link, as the window should act on it. What it carries is a device
/// *name*, resolved against the config on this machine - never an address, because a
/// link that could name a host is a link that dials a stranger's box and asks for a
/// password. A name that isn't here is refused rather than guessed at.
///
/// ponytail: macOS only. The scheme is registered in `Info.plist` and arrives as
/// `RunEvent::Opened`; Windows and Linux want a registry key or a `.desktop` file
/// written at install time and a single-instance hand-off, which is a plugin's worth
/// of work for the same two lines of behaviour.
fn link_target(url: &str) -> Option<String> {
    let name = link_name(url)?;
    // The exact name, never `resolve`'s unique-substring match. The palette guesses
    // because you are watching it guess; a link in an alert is read once and acted on,
    // and `patchbay://db` quietly opening `db-prod` is the wrong box at 3am.
    read().ok()?.contains_key(&name).then_some(name)
}

/// The name a link carries, or None. Split out from the lookup so the part that
/// decides what a URL may say is testable without a config on disk.
fn link_name(url: &str) -> Option<String> {
    let rest = url.strip_prefix("patchbay://")?.trim_end_matches('/');
    // The first segment and nothing else: no path, no query, no fragment.
    let name = percent_decode(rest.split(['/', '?', '#']).next()?);
    // Structurally not an address, rather than incidentally not one. `resolve` would
    // refuse a host anyway by simply not finding it, but a link that cannot even be
    // *shaped* like `user@host:22` is the property worth being able to point at.
    // A dot is allowed: `web.example` is an ordinary jack name, and it is the `@` and
    // the `:` that keep a link from being shaped like `user@host:22`. Being unable to
    // name a host was never the guarantee anyway - `link_target` matches this against
    // the config exactly, so a link can only ever open a device you already have.
    let ok = !name.is_empty()
        && !name.contains(['@', ':', ' ', '\\', '/'])
        && !name.chars().any(char::is_control);
    ok.then_some(name)
}

/// A link is a URL, so a name with a space in it arrives percent-encoded. Only the
/// escapes - nothing here decides anything, `resolve` does, and a name it doesn't know
/// is refused.
fn percent_decode(s: &str) -> String {
    // Bytes, then one decode at the end: an escape is a *byte* of UTF-8, so `%C3%A9`
    // is one letter and pushing each half as a char would give two of the wrong ones.
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

/// A link that arrived before the window was listening. macOS launches the app to
/// deliver one, so the first link of a cold start lands while `boot.js` is still
/// fetching the list - `emit` into a window with no listener yet goes nowhere.
#[derive(Default)]
struct PendingLink(std::sync::Mutex<Option<String>>);

/// Asked for once, on boot. Anything after that arrives as the event instead.
#[tauri::command]
fn take_link(pending: tauri::State<'_, PendingLink>) -> Option<String> {
    pending.0.lock().unwrap().take()
}

#[tauri::command]
fn config_path() -> String {
    patchbay::config_path().display().to_string()
}

#[tauri::command]
fn open_config() -> Result<(), String> {
    os_open(patchbay::config_path().as_os_str())
}

/// The disk image someone dragged us out of, still mounted. Nothing in macOS ejects
/// it - Finder has no hook for "the drag is done" - so the copy that came out of it
/// is the only thing that knows it is there and knows it is ours.
///
/// Never while running *from* the image: that is someone trying the app before
/// installing it, and pulling the volume out from under a live process is worse than
/// a stray icon. `spawn` and not `status`, because a volume Finder still has open
/// simply stays mounted, and that is not a startup's problem to report.
#[cfg(target_os = "macos")]
fn eject_install_image() {
    let volume = std::path::Path::new("/Volumes/patchbay");
    let ours = volume.join("patchbay.app").is_dir();
    let running_from_it = std::env::current_exe().map(|p| p.starts_with(volume)).unwrap_or(true);
    if ours && !running_from_it {
        let _ = std::process::Command::new("/usr/bin/hdiutil")
            .args(["detach", "/Volumes/patchbay", "-quiet"])
            .spawn();
    }
}

#[tauri::command]
fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// What the pill offers: the version, and the release notes that came with it.
#[derive(Serialize)]
struct Offer {
    version: String,
    /// The CHANGELOG section for that version, as the release carries it. The update
    /// *signature* covers the archive and not this, so it is text from outside like
    /// any other and the window escapes it - and it is cut short here, because a pill
    /// is not the place to render somebody's essay.
    notes: Option<String>,
}

/// The version waiting on the update endpoint, or `None` when this is the latest.
/// Signature-checked by the plugin against the public key in `tauri.conf.json`, so a
/// tampered feed or archive fails here rather than installing.
///
/// An unreachable endpoint is an error here and swallowed by the launch check, which
/// has nothing worth interrupting anyone about - but a check someone *asked* for has
/// to be able to say why it found nothing.
#[tauri::command]
async fn update_check(app: tauri::AppHandle) -> Result<Option<Offer>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(found.map(|u| Offer {
        version: u.version,
        // Characters, not bytes: `truncate` panics mid-codepoint, and a changelog
        // with a `▸` in it would be the thing that found that out.
        notes: u.body.map(|b| b.chars().take(4000).collect()),
    }))
}

/// Checked a second time rather than parked between the two commands: an `Update`
/// isn't `Send`, and a second round trip on the one path where the user is already
/// waiting for a download costs nothing.
#[tauri::command]
async fn update_install(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    use tauri_plugin_updater::UpdaterExt;
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
                // A server that won't say how big it is leaves the pill on its
                // indeterminate pulse rather than inventing a number.
                let Some(total) = total.filter(|t| *t > 0) else { return };
                let pct = (got * 100 / total).min(100) as u8;
                // One event per whole percent: the download is hundreds of chunks
                // and a bar 200px wide can only move a hundred times anyway.
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

/// The restart, once the new bundle is in place. Split from the install because it
/// takes every live session with it, so it waits for a second click. (Windows is the
/// exception nobody clicks: its installer takes the process down during the install
/// above, so the pill never gets as far as offering this.)
#[tauri::command]
fn update_restart(app: tauri::AppHandle) {
    // A restart is not `RunEvent::Exit`, so the tunnels have to be closed by hand -
    // otherwise `ssh -N -L` outlives us and the new process can't have its ports back.
    app.state::<rdp::SharedTunnels>().close_all();
    app.restart();
}

fn main() {
    use tauri_plugin_window_state::StateFlags;
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        // An app that forgets where it was is one you re-arrange every morning.
        // Not DECORATIONS: the title bar is `Overlay` from tauri.conf.json, and a
        // restored decoration state would fight it.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED | StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .manage(pty::Shared::default())
        .manage(rdp::SharedTunnels::default())
        .manage(rdp_session::Shared::default())
        .manage(PendingLink::default())
        .setup(|app| {
            // Spaces were extra config files, from when a shared list had to be one.
            // Anything still in `spaces/` is folded into the list on the way past, so
            // nobody opens the window to find half their devices missing.
            match config::fold_spaces_at(&patchbay::config_path()) {
                Ok(0) => {}
                Ok(n) => println!("patchbay: folded {n} device(s) out of spaces/ into your list"),
                Err(e) => eprintln!("patchbay: couldn't fold spaces/ in: {e}"),
            }
            #[cfg(target_os = "macos")]
            {
                eject_install_image();
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let w = app.get_webview_window("main").unwrap();
                // Sidebar, not HudWindow: HUD material is built for floating panels
                // and is transparent enough that sidebar text washes out over a
                // bright desktop. This is the material Finder and Mail use.
                let _ = apply_vibrancy(&w, NSVisualEffectMaterial::Sidebar, None, Some(12.0));
            }
            // The default menu bar, minus Close Window. ⌘W belongs to the tab strip,
            // and a menu accelerator is a native key equivalent - macOS closed the
            // window before the page was ever asked, so the handler in boot.js never
            // ran and the whole app went with the tab. Spelled out rather than filtered
            // out of `Menu::default`, because a predefined item's id is a counter and
            // there is nothing but its English label to recognise it by. Edit is here
            // for its own sake: without it ⌘C and ⌘V stop working in the webview.
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
                let h = app.handle();
                // Where a Mac user looks for it, and the only entry point on this
                // platform that doesn't need the settings sheet open first. The id is
                // matched in the handler below - a predefined item's id is a counter,
                // but ours is ours.
                let check = MenuItem::with_id(h, "check-update", "Check for Updates…", true, None::<&str>)?;
                let about = SubmenuBuilder::new(h, "patchbay")
                    .about(None)
                    .separator()
                    .item(&check)
                    .separator()
                    .services()
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;
                let edit = SubmenuBuilder::new(h, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                let window = SubmenuBuilder::new(h, "Window")
                    .minimize()
                    .maximize()
                    .separator()
                    .fullscreen()
                    .build()?;
                app.set_menu(MenuBuilder::new(h).items(&[&about, &edit, &window]).build()?)?;
                // The window owns what a check looks like, so the menu only says it
                // was asked for.
                app.on_menu_event(|app, event| {
                    if event.id() == "check-update" {
                        use tauri::Emitter;
                        let _ = app.emit("menu:check-update", ());
                    }
                });
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            jacks, connect, probe, config_path, open_config,
            save_jack, delete_jack, rename_group, delete_group, notes, save_note,
            open_url, open_link,

            settings, save_settings, set_theme, colors, save_color, defaults, save_defaults, ssh_keys,
            ssh_hosts, royal_hosts, ssh_leftovers, clean_ssh_leftovers,
            open_web_view, place_web_view, close_web_view, web_check, web_trust, web_cert, web_trust_cert,
            open_rdp, open_vnc, open_rdp_session, close_rdp_session, rdp_input,
            tunnels, close_tunnel, open_forwards,
            open_session, open_task, write_session, resize_session, close_session,
            sftp_ls, sftp_get, sftp_put, sftp_edit, sftp_open, sftp_ready,
            open_master, open_full_disk_access,
            app_version, update_check, update_install, update_restart,
            take_link
        ])
        .build(tauri::generate_context!())
        .expect("error while building patchbay")
        .run(|handle, event| {
            // `patchbay://web-01` from a runbook, a ticket or an alert. Held as well as
            // emitted: a cold start delivers the link before the window is listening.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                use tauri::Emitter;
                if let Some(name) = urls.iter().find_map(|u| link_target(u.as_str())) {
                    *handle.state::<PendingLink>().0.lock().unwrap() = Some(name.clone());
                    let _ = handle.emit("open:link", name);
                }
            }
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

    /// A link is the one way into this app that a *web page* can reach. So what it may
    /// carry is a device name and nothing else: a link that could name a host would be
    /// a link that dials a stranger's box and asks you for a password. `link_target`
    /// then matches that name against the config exactly, so the worst a link can do
    /// is open something already on this machine, or nothing.
    #[test]
    fn a_link_carries_a_device_name_and_never_a_login() {
        assert_eq!(super::link_name("patchbay://web-01").as_deref(), Some("web-01"));
        assert_eq!(super::link_name("patchbay://web-01/").as_deref(), Some("web-01"));
        // Anything after the name is not part of it.
        assert_eq!(super::link_name("patchbay://db/etc/passwd?x=1").as_deref(), Some("db"));
        assert_eq!(super::link_name("patchbay://a%2Db").as_deref(), Some("a-b"));
        // An escape is a byte of UTF-8, not a character - `caf%C3%A9` is one letter.
        assert_eq!(super::link_name("patchbay://caf%C3%A9").as_deref(), Some("café"));

        // A dotted name is a name people actually use, and it is `link_target` that
        // decides whether it exists here.
        assert_eq!(super::link_name("patchbay://web.example").as_deref(), Some("web.example"));

        for bad in [
            "patchbay://root@evil.example",
            "patchbay://10.0.0.1:22",
            "patchbay://",
            "patchbay://a b",
            "https://evil.example",
            "file:///etc/passwd",
        ] {
            assert_eq!(super::link_name(bad), None, "{bad:?} should carry no name");
        }
    }

    #[test]
    fn a_check_runs_here_when_it_can_and_on_the_hop_when_it_cannot() {
        let j = super::patchbay::parse(CHAIN).unwrap();

        // Nothing in the way: straight at the host.
        let (p, a) = task_argv("ping", "plain", &j).unwrap();
        assert_eq!(p, "ping");
        assert!(a.contains(&"10.0.0.4".to_string()), "got {a:?}");

        // Behind a bastion, so it runs there - and the hop's own tunnel is left out,
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

    /// A NAS is reached by its IP, so its certificate names something else and never
    /// will match. "Show it anyway" has to survive a restart, or the answer is asked
    /// for again every single time and stops being read.
    #[test]
    fn showing_a_device_anyway_is_remembered() {
        let dir = std::env::temp_dir().join(format!("patchbay-webtrust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("web_trusted");
        let url = "https://10.0.0.251:5001/";

        assert!(!super::web_trusted_at(&store, url), "trusted before anyone said so");
        super::web_trust_at(&store, url).unwrap();
        assert!(super::web_trusted_at(&store, url));
        // Saying it twice is not two lines, and a different device is still unanswered.
        super::web_trust_at(&store, url).unwrap();
        assert_eq!(std::fs::read_to_string(&store).unwrap().lines().count(), 1);
        assert!(!super::web_trusted_at(&store, "https://10.0.0.9:5001/"));
    }

    /// The webview can load cleartext at all only because `Info.plist` turns ATS off
    /// for web content, which is broader than we want. This is the narrowing: a page
    /// on the LAN, never one on the public internet.
    #[test]
    fn cleartext_is_for_your_own_network_only() {
        for ours in [
            "10.0.0.251", "10.0.0.4", "172.16.3.9", "172.31.255.1",
            "127.0.0.1", "localhost", "169.254.1.1", "nas", "nas.local", "::1", "fe80::1", "fd00::1",
            // Tailscale hands out 100.64/10, and a tailnet is someone's own network
            // in every sense that matters here.
            "100.64.0.1", "100.101.102.103", "100.127.255.254",
        ] {
            assert!(super::is_private_host(ours), "{ours:?} is on your own network");
        }
        for theirs in [
            "example.com", "8.8.8.8", "172.32.0.1", "172.15.0.1", "1.1.1.1",
            "evil.example.co.uk", "2606:4700::1111",
            // Either side of the CGNAT block is the public internet.
            "100.63.255.255", "100.128.0.1",
        ] {
            assert!(!super::is_private_host(theirs), "{theirs:?} is not");
        }
    }

    /// A username reaches the desktop opener inside the url, where an `@` would end
    /// the userinfo and everything after it would be read as the host.
    #[test]
    fn a_vnc_url_carries_only_a_name_it_can_carry() {
        assert_eq!(super::vnc_url("10.0.0.9:5900", Some("leon")).unwrap(), "vnc://leon@10.0.0.9:5900");
        assert_eq!(super::vnc_url("10.0.0.9:5900", Some("  ")).unwrap(), "vnc://10.0.0.9:5900");
        // Dropped, not refused: the viewer will ask for the name itself.
        assert_eq!(
            super::vnc_url("127.0.0.1:5901", Some("me@evil.example")).unwrap(),
            "vnc://127.0.0.1:5901"
        );
        assert!(super::vnc_url("10.0.0.9:5900/../x", None).is_err());
        assert!(super::vnc_url("bad host\u{7}", None).is_err());
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
