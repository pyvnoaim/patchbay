//! A device's web UI: opened in the browser, or as a tab (a child webview above the
//! page). The tab has no certificate interstitial, so this module also explains why a
//! page stayed blank and can hand a certificate to the system trust store.
//!
//! The child webview gets no capability, and must not: its page is a remote origin
//! that matches nothing in `capabilities/`, which is all that keeps an appliance's
//! login page away from `delete_jack`.

use super::{blocking, load_jacks, os_open};
use crate::patchbay::{self, is_web_url};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Manager;

const NOT_WEB: &str = "only http:// and https:// urls can be opened";

/// A device's `url`, resolved and checked.
fn web_url_of(name: &str) -> Result<(String, String), String> {
    let jacks = load_jacks()?;
    let resolved = patchbay::resolve(name, &jacks)?;
    let url = jacks
        .get(&resolved)
        .and_then(|j| j.url.as_deref())
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| format!("\"{resolved}\" has no url"))?
        .to_string();
    if !is_web_url(&url) {
        return Err(NOT_WEB.into());
    }
    Ok((resolved, url))
}

/// Open a device's web UI in the browser, the fallback with a certificate interstitial.
#[tauri::command]
pub async fn open_url(name: String) -> Result<String, String> {
    let (_, url) = web_url_of(&name)?;
    os_open(url.as_ref())?;
    Ok(url)
}

/// A link clicked in terminal output. Text a remote host wrote, so it goes nowhere
/// but a browser, and only if it is http(s).
#[tauri::command]
pub fn open_link(url: String) -> Result<String, String> {
    if !is_web_url(&url) {
        return Err(format!("\"{url}\" isn't a web address"));
    }
    os_open(url.as_ref())?;
    Ok(url)
}

fn web_label(id: u32) -> String {
    format!("webtab-{id}")
}

/// Open a device's web UI as a tab. Cleartext to a public address goes to the browser
/// instead; see `is_private_host`.
///
/// No reachability check here: it cost a round trip before anything appeared. The
/// view goes up now and `web_check` explains a blank one afterwards. `on_navigation`
/// reports where the page really ends up, because a Synology's http port is a
/// JavaScript redirect to its https one, which no http client can see.
#[tauri::command]
pub async fn open_web_view(
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
        os_open(url.as_ref())?;
        return Err(format!(
            "\"{url}\" is plain http to a public address - patchbay opens cleartext \
             only on your own network, so it opened in your browser instead"
        ));
    }
    let window = app.get_window("main").ok_or("the main window has gone")?;
    let reporter = app.clone();
    let loaded = app.clone();
    let at = tauri::LogicalPosition::new(x, y);
    let size = tauri::LogicalSize::new(width.max(1.0), height.max(1.0));
    let home = parsed.host_str().map(str::to_owned);
    let build = move || {
        tauri::webview::WebviewBuilder::new(web_label(id), tauri::WebviewUrl::External(parsed))
            // Without a handler WebKit drops `window.open` on the floor, and that is
            // how Proxmox opens its upgrade shell and a VM's console. The device's own
            // popups get a window sharing the tab's session; a link off the device
            // goes to the browser, like terminal links do.
            .on_new_window(move |to, _| {
                use tauri::webview::NewWindowResponse;
                if !is_web_url(to.as_str()) {
                    return NewWindowResponse::Deny;
                }
                if to.host_str() == home.as_deref() {
                    return NewWindowResponse::Allow;
                }
                let _ = os_open(to.as_str().as_ref());
                NewWindowResponse::Deny
            })
            .on_navigation(move |to| {
                use tauri::Emitter;
                // Only a page worth re-checking. A login flow navigates to
                // `about:blank` and `blob:` on its way, and reporting one of those
                // put "only http:// and https:// urls can be opened" over a tab
                // that was loading fine. Never false: this reports, never blocks.
                if is_web_url(to.as_str()) {
                    let _ = reporter.emit(&format!("web-nav:{id}"), to.to_string());
                }
                true
            })
            // A page that rendered is the only proof that beats a preflight. WebKit
            // finishes no navigation it refused a certificate for, so this arriving
            // means the tab is fine whatever `web_check` would have predicted.
            .on_page_load(move |_, payload| {
                use tauri::Emitter;
                if payload.event() == tauri::webview::PageLoadEvent::Finished {
                    let _ = loaded.emit(&format!("web-load:{id}"), payload.url().to_string());
                }
            })
    };

    // With the extension up the whole build moves to the main thread: a
    // `WKWebViewConfiguration` can only be made there and cannot be sent, and WebKit
    // reads the controller off it when the view is created rather than after.
    #[cfg(target_os = "macos")]
    if crate::webext::running() {
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let w = window.clone();
        app.run_on_main_thread(move || {
            let mut b = build();
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                if let Some(conf) = crate::webext::tab_configuration(mtm) {
                    b = b.with_webview_configuration(conf);
                }
            }
            let _ = tx.send(
                w.add_child(b, at, size)
                    .map(|v| {
                        // The extension learns of a tab through its WKWebView, which
                        // only `with_webview` hands out.
                        let _ = v.with_webview(move |p| crate::webext::open_tab(id, p.inner()));
                    })
                    .map_err(|e| e.to_string()),
            );
        })
        .map_err(|e| format!("could not reach the main thread: {e}"))?;
        let out = tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(20))
                .unwrap_or_else(|_| Err("the tab never opened".into()))
        })
        .await
        .map_err(|e| e.to_string())?;
        out.map_err(|e| format!("\"{resolved}\": {e}"))?;
        return Ok(url);
    }

    window
        .add_child(build(), at, size)
        .map_err(|e| format!("\"{resolved}\": {e}"))?;
    Ok(url)
}

/// Move and size a web tab, in logical pixels. A child webview obeys no CSS of ours -
/// not `hidden`, not z-index, not a sheet - so anything opening over it calls this
/// with a zero size first.
///
/// Zero size *and* `hide()`: a 0x0 WKWebView still drew, which is what put the
/// settings sheet behind a Synology's login page. Only a tab whose page had failed
/// looked right, because that one was already sized away for its own reasons.
#[tauri::command]
pub fn place_web_view(
    app: tauri::AppHandle,
    id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // A tab that has already closed is an ordinary race with a resize, not an error.
    let Some(w) = app.get_webview(&web_label(id)) else {
        return Ok(());
    };
    if width <= 0.0 || height <= 0.0 {
        return w.hide().map_err(|e| e.to_string());
    }
    w.set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    w.set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    // The tab in front is the one Bitwarden fills. Queued rather than called: the
    // extension's state is the main thread's, whichever thread this runs on.
    #[cfg(target_os = "macos")]
    if crate::webext::running() {
        let _ = app.run_on_main_thread(move || crate::webext::activate(id));
    }
    w.show().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_web_view(app: tauri::AppHandle, id: u32) {
    #[cfg(target_os = "macos")]
    if crate::webext::running() {
        let _ = app.run_on_main_thread(move || crate::webext::close_tab(id));
    }
    if let Some(w) = app.get_webview(&web_label(id)) {
        let _ = w.close();
    }
}

/// Why a blank tab is blank. Takes a url rather than a device because the one worth
/// checking is often a redirect's destination.
#[tauri::command]
pub async fn web_check(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err(NOT_WEB.into());
    }
    if web_trusted(&url) {
        return Ok(());
    }
    blocking(move || web_reachable(&url)).await
}

/// One request of our own, to turn a silent blank page into a reason.
fn web_reachable(url: &str) -> Result<(), String> {
    // 15s, not less: a NAS waking from hibernation drops the first SYN and takes its
    // time, and "not answering" arriving late beats it arriving wrong.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("no http client: {e}"))?;
    client.get(url).send().map(|_| ()).map_err(|e| {
        // reqwest's Display stops at "error sending request"; the cause is only ever
        // in the source chain.
        let mut why = e.to_string();
        let mut cause: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
        while let Some(c) = cause {
            why = format!("{why}: {c}");
            cause = c.source();
        }
        let cert_problem = [
            "certificate",
            "UnknownIssuer",
            "NotValidForName",
            "CertExpired",
        ]
        .iter()
        .any(|s| why.contains(s));
        if cert_problem {
            format!(
                "\"{url}\" uses a certificate this machine doesn't trust, so a window \
                 here would show nothing - trust it on this machine and it opens in the app"
            )
        } else {
            format!("\"{url}\": {why}")
        }
    })
}

/// Cleartext is LAN-only. `Info.plist` has to turn ATS off for web content to allow
/// http at all, and Apple offers nothing narrower that covers `192.168.x.x`, so the
/// narrowing happens here.
fn is_private_host(host: &str) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            // 100.64/10 is carrier-grade NAT, which is what Tailscale hands out.
            // `Ipv4Addr::is_shared` would say so but is still unstable.
            let o = v4.octets();
            let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
            v4.is_private()
                || cgnat
                || v4.is_loopback()
                || v4.is_link_local()
                || v4 == Ipv4Addr::UNSPECIFIED
        }
        Ok(IpAddr::V6(v6)) => {
            // Unique-local (fc00::/7), link-local (fe80::/10) and ::1.
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        // `.local` is mDNS, and a name with no dot can only resolve on this network.
        Err(_) => {
            let h = host.to_ascii_lowercase();
            let h = h.strip_suffix('.').unwrap_or(&h);
            h == "localhost"
                || h.ends_with(".local")
                || h.ends_with(".home.arpa")
                || !h.contains('.')
        }
    }
}

// The trust store: urls whose certificate warning was waived on this machine. Kept
// beside the config, not in it, because it is this machine's answer about one device.

fn web_trust_store() -> PathBuf {
    patchbay::config_path().with_file_name("web_trusted")
}

fn web_trusted(url: &str) -> bool {
    web_trusted_at(&web_trust_store(), url)
}

/// A waiver is about a device's certificate, so it is keyed by origin. Keyed by the
/// whole url it missed constantly: the webview navigates to `.../` where the config
/// says `...:8006`, `web-nav` re-checks that, and the panel came back over a page the
/// trusted certificate had just made load. Reading old full-url lines through the same
/// key keeps a store written before this.
fn trust_key(url: &str) -> String {
    tauri::Url::parse(url.trim())
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|_| url.trim().to_string())
}

fn web_trusted_at(store: &Path, url: &str) -> bool {
    let key = trust_key(url);
    std::fs::read_to_string(store)
        .unwrap_or_default()
        .lines()
        .any(|l| trust_key(l) == key)
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
    writeln!(f, "{}", trust_key(url)).map_err(|e| format!("{}: {e}", store.display()))
}

/// "Show it anyway", remembered. This can't make the webview accept a certificate;
/// it stops our own stricter check from hiding a page the webview renders fine once
/// the certificate is trusted on this machine.
#[tauri::command]
pub fn web_trust(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err(NOT_WEB.into());
    }
    web_trust_at(&web_trust_store(), &url)
}

// A hosted browser extension is the web tab's story too, so its thin surface sits here
// rather than in a file of its own; `webext.rs` is where the work is.

/// Whether the toggle is offerable at all: `WKWebExtension` is macOS 15.4 and newer.
#[tauri::command]
pub fn webext_supported() -> bool {
    crate::webext::supported()
}

/// What the installed Bitwarden extension turns out to be. Settings shows the answer
/// beside the toggle, because a build `WKWebExtension` won't take has to say so rather
/// than leave a switch that quietly does nothing.
#[tauri::command]
pub async fn webext_inspect(app: tauri::AppHandle) -> Result<crate::webext::Package, String> {
    let path = crate::webext::package()?;
    crate::webext::inspect(&app, path).await
}

/// Every device url in the list, as the match patterns the extension is allowed to see.
/// This is the whole of its reach, so it is computed from the list and nowhere else.
fn web_hosts() -> Result<Vec<String>, String> {
    let jacks = load_jacks()?;
    let mut seen: Vec<String> = jacks
        .values()
        .filter_map(|j| j.url.as_deref())
        .filter_map(crate::webext::host_pattern)
        .collect();
    seen.sort();
    seen.dedup();
    Ok(seen)
}

/// Start the extension over the web tabs. Called when the toggle goes on, at launch when
/// it already was, and on every opening of Settings, where it reports what is running.
#[tauri::command]
pub async fn webext_start(app: tauri::AppHandle) -> Result<crate::webext::Package, String> {
    let hosts = web_hosts()?;
    let out = crate::webext::load(&app, hosts).await;
    // A failure at launch only ever reaches a pill; the log is where to find it after.
    if let Err(e) = &out {
        eprintln!("patchbay: bitwarden start failed: {e}");
    }
    out
}

/// Run `work` on the main thread, where every WebKit object lives, and wait for its
/// answer without making the main thread wait for anything.
async fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(work());
    })
    .map_err(|e| format!("could not reach the main thread: {e}"))?;
    blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap_or_else(|_| Err("Bitwarden didn't answer".into()))
    })
    .await
}

#[tauri::command]
pub async fn webext_stop(app: tauri::AppHandle) -> Result<(), String> {
    on_main(&app, crate::webext::unload).await
}

/// The key button: a fill, or with `popup` Bitwarden itself. `id` is the web tab it
/// sits on, or `None` for the toolbar's key, which is the vault rather than a page.
/// The rect is the button's, so whatever Bitwarden shows hangs from it.
#[tauri::command]
pub async fn webext_key(
    app: tauri::AppHandle,
    id: Option<u32>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    popup: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let window = app.get_window("main").ok_or("the main window has gone")?;
        on_main(&app, move || {
            let view = window.ns_view().map_err(|e| e.to_string())?;
            crate::webext::key(view, id, (x, y, width, height), popup)
        })
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, id, x, y, width, height, popup);
        Err("Bitwarden only runs on macOS".into())
    }
}

/// What the user is asked to trust, in the words the OS dialog would use.
#[derive(Serialize)]
struct CertFacts {
    subject: String,
    issuer: String,
    expires: String,
    /// SHA-256 over the DER, the digest every other tool prints.
    fingerprint: String,
    /// The device's own signing certificate rather than the one it is serving today.
    /// The window says so before asking: it is a wider answer than one certificate.
    ca: bool,
}

/// The certificate a device should be trusted by, fetched without judging it. The OS
/// does the judging, and it can't judge what it hasn't been shown.
fn peer_cert(url: &str) -> Result<(String, Vec<u8>, bool), String> {
    use std::io::Write as _;
    use std::net::ToSocketAddrs as _;
    use tokio_rustls::rustls;

    let u = tauri::Url::parse(url).map_err(|e| format!("\"{url}\": {e}"))?;
    let host = u
        .host_str()
        .ok_or_else(|| format!("\"{url}\" has no host"))?
        .to_string();
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
        .with_custom_certificate_verifier(std::sync::Arc::new(
            crate::rdp_session::verifier::AcceptAny,
        ))
        .with_no_client_auth();
    let name = host
        .clone()
        .try_into()
        .map_err(|_| format!("\"{host}\" isn't a usable server name"))?;
    let client = rustls::ClientConnection::new(std::sync::Arc::new(config), name)
        .map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(client, stream);
    // The handshake hasn't reached the peer certificate until something is flushed.
    tls.flush().map_err(|e| format!("{host}: {e}"))?;

    let chain: Vec<Vec<u8>> = tls
        .conn
        .peer_certificates()
        .map(|c| c.iter().map(|c| c.as_ref().to_vec()).collect())
        .unwrap_or_default();
    let (der, ca) = anchor(chain).ok_or_else(|| format!("\"{host}\" sent no certificate"))?;
    Ok((host, der, ca))
}

/// Which certificate of a chain is the one worth trusting, and whether that is the
/// device's signer rather than the certificate it is serving.
///
/// The appliance's own CA rather than its leaf: a NAS regenerates the leaf on a
/// firmware update, and trust pinned to that leaf dies with it - the tab blanks and the
/// panel asks about a device already answered for. Only a *self-signed* tail counts,
/// which is the appliance's own root and never a public CA's intermediate: `-s <host>`
/// scopes the trust to this device, but a public intermediate has no business in
/// anyone's keychain. Anything else is trusted as the leaf it is, as before.
///
/// macOS only, because that scoping is: Windows' `certutil -addstore Root` has no
/// per-host policy, so a CA landing there could sign for anything. And the chain has to
/// link - the tail is the signer of what the device is serving, or it is something the
/// device appended and has no more claim on the keychain than any other stranger.
fn anchor(chain: Vec<Vec<u8>>) -> Option<(Vec<u8>, bool)> {
    use x509_cert::der::Decode as _;
    let parsed: Vec<_> = chain
        .iter()
        .map(|d| x509_cert::Certificate::from_der(d).ok())
        .collect();
    let signed_by = |pair: &[Option<x509_cert::Certificate>]| match (&pair[0], &pair[1]) {
        (Some(c), Some(up)) => c.tbs_certificate.issuer == up.tbs_certificate.subject,
        _ => false,
    };
    let signs_itself = parsed.last().is_some_and(|c| {
        c.as_ref()
            .is_some_and(|c| c.tbs_certificate.subject == c.tbs_certificate.issuer)
    });
    let own_ca = cfg!(target_os = "macos")
        && chain.len() > 1
        && signs_itself
        && parsed.windows(2).all(signed_by);
    match own_ca {
        true => chain.last().cloned().map(|tail| (tail, true)),
        false => chain.into_iter().next().map(|leaf| (leaf, false)),
    }
}

fn cert_facts(der: &[u8], ca: bool) -> Result<CertFacts, String> {
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
        ca,
    })
}

/// Fetch and describe a device's certificate, trusting nothing. Separate from
/// `web_trust_cert` so the window can show it first and ask second.
#[tauri::command]
pub async fn web_cert(url: String) -> Result<serde_json::Value, String> {
    if !is_web_url(&url) {
        return Err(NOT_WEB.into());
    }
    blocking(move || {
        let (_, der, ca) = peer_cert(&url)?;
        serde_json::to_value(cert_facts(&der, ca)?).map_err(|e| e.to_string())
    })
    .await
}

/// Trust a device's certificate on this machine, so the webview will load its page.
#[tauri::command]
pub async fn web_trust_cert(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err(NOT_WEB.into());
    }
    blocking(move || {
        let (host, der, _) = peer_cert(&url)?;
        trust_cert(&host, &der)?;
        // Our own check still refuses a mismatched name, so record the waiver too or
        // the panel comes straight back for a page that now loads.
        web_trust_at(&web_trust_store(), &url)
    })
    .await
}

/// Hand the certificate to macOS the way the browser's "Always trust" does. `security`
/// raises the system's own password prompt; we never write trust settings ourselves.
/// `-s <host>` scopes the trust to this device, so a CA certificate handed out by an
/// appliance never becomes a trusted root for everything. `-e hostnameMismatch` is
/// the error being forgiven: an appliance reached by IP has a certificate naming
/// something else.
#[cfg(target_os = "macos")]
fn trust_cert(host: &str, der: &[u8]) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("patchbay-cert-{}.der", std::process::id()));
    std::fs::write(&path, der).map_err(|e| format!("{}: {e}", path.display()))?;
    let keychain = dirs::home_dir()
        .ok_or("no home directory")?
        .join("Library/Keychains/login.keychain-db");
    let out = std::process::Command::new("/usr/bin/security")
        .args([
            "add-trusted-cert",
            "-r",
            "trustAsRoot",
            "-p",
            "ssl",
            "-e",
            "hostnameMismatch",
        ])
        .args(["-s", host, "-k"])
        .arg(&keychain)
        .arg(&path)
        .output()
        .map_err(|e| format!("could not run security: {e}"));
    let _ = std::fs::remove_file(&path);
    trust_outcome(out?)
}

/// Windows keeps its own store and WebView2 reads it. `-user` keeps this to one
/// account, because the machine store needs an administrator. Windows has no per-host
/// name-mismatch waiver, so WebView2 may still refuse a certificate trusted here.
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
    trust_outcome(out?)
}

/// Linux has no one store the webview reads, and writing to the system bundle is the
/// distribution's business.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn trust_cert(_host: &str, _der: &[u8]) -> Result<(), String> {
    Err("trusting a certificate from here isn't supported on this system - accept it in your browser instead".into())
}

/// A cancelled system prompt is a decision, not a failure to explain.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn trust_outcome(out: std::process::Output) -> Result<(), String> {
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(match why.is_empty() {
        true => "the certificate wasn't trusted".to_string(),
        false => format!("the certificate wasn't trusted: {why}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_a_device_anyway_is_remembered() {
        let dir = std::env::temp_dir().join(format!("patchbay-webtrust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("web_trusted");
        let url = "https://192.168.1.20:5001/";

        assert!(
            !web_trusted_at(&store, url),
            "trusted before anyone said so"
        );
        web_trust_at(&store, url).unwrap();
        assert!(web_trusted_at(&store, url));
        // Saying it twice is one line, and a different device is still unanswered.
        web_trust_at(&store, url).unwrap();
        assert_eq!(std::fs::read_to_string(&store).unwrap().lines().count(), 1);
        assert!(!web_trusted_at(&store, "https://192.168.1.9:5001/"));
        // The webview navigates where the config only named an origin, and a redirect
        // lands on a path. Same certificate, same answer, still one line.
        for same in [
            "https://192.168.1.20:5001",
            "https://192.168.1.20:5001/webman/index.cgi",
        ] {
            assert!(web_trusted_at(&store, same), "{same} asked again");
            web_trust_at(&store, same).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&store).unwrap().lines().count(), 1);
        // A different port is a different certificate.
        assert!(!web_trusted_at(&store, "https://192.168.1.20:5000/"));
    }

    #[test]
    fn only_a_self_signed_tail_is_trusted_over_the_leaf() {
        let leaf = b"leaf".to_vec();
        let issuer = b"issuer".to_vec();
        // Nothing that can't be read as a self-signed certificate that signed the one
        // below it takes the leaf's place - every public CA's intermediate, a tail the
        // device just appended, and this junk.
        assert_eq!(
            anchor(vec![leaf.clone(), issuer]),
            Some((leaf.clone(), false))
        );
        assert_eq!(anchor(vec![leaf.clone()]), Some((leaf, false)));
        assert_eq!(anchor(vec![]), None);
    }

    #[test]
    fn cleartext_is_for_your_own_network_only() {
        for ours in [
            "192.168.1.20",
            "10.0.0.4",
            "172.16.3.9",
            "172.31.255.1",
            "127.0.0.1",
            "localhost",
            "169.254.1.1",
            "nas",
            "nas.local",
            "::1",
            "fe80::1",
            "fd00::1",
            // Tailscale's 100.64/10 is someone's own network too.
            "100.64.0.1",
            "100.101.102.103",
            "100.127.255.254",
        ] {
            assert!(is_private_host(ours), "{ours:?} is on your own network");
        }
        for theirs in [
            "example.com",
            "8.8.8.8",
            "172.32.0.1",
            "172.15.0.1",
            "1.1.1.1",
            "evil.example.co.uk",
            "2606:4700::1111",
            "100.63.255.255",
            "100.128.0.1",
        ] {
            assert!(!is_private_host(theirs), "{theirs:?} is not");
        }
    }

    /// reqwest's Display stops at the url; the reason lives in the source chain.
    #[test]
    fn a_web_ui_that_wont_load_says_why_not_just_which() {
        let err = web_reachable("https://127.0.0.1:1").unwrap_err();
        assert!(err.contains("127.0.0.1:1"), "{err}");
        assert!(
            err.to_lowercase().contains("refused"),
            "the cause chain wasn't walked, so the message names no cause: {err}"
        );
    }
}
