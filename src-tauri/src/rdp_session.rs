//! Remote desktop in the window, the way `pty.rs` does terminals.
//!
//! This is the one place patchbay speaks a protocol itself rather than handing off:
//! IronRDP is a pure-Rust RDP stack, so there is still no FreeRDP, no C dependency
//! and no embedded graphics toolkit — we decode to a framebuffer and let the webview
//! paint it on a `<canvas>`. `rdp.rs` keeps the handoff to the system client, which
//! is still what "Open in Windows App" does.
//!
//! A host behind a bastion is reached through the local port `rdp.rs` already
//! forwards over the ssh chain, so this module only ever dials somewhere directly.

use ironrdp::connector::{self, ConnectionResult, Credentials};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::PerformanceFlags;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::CliprdrClient;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use tauri::Emitter as _;
use tokio_rustls::rustls;

/// A decoded rectangle, ready to be blitted onto the canvas. RGBA so the webview can
/// hand it straight to `putImageData` without touching a pixel.
pub struct Tile {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// What the window can send into a running session. Closing isn't one of them —
/// dropping the sender does that, so there is no way to leave a socket open by
/// forgetting to send something.
pub enum Input {
    Move { x: u16, y: u16 },
    Button { button: u8, down: bool },
    Wheel { delta: i16 },
    /// A PC/AT scancode, already translated from the browser's `KeyboardEvent.code`
    /// on the JS side — that mapping is a table, and a table belongs where the
    /// event names are.
    Key { scancode: u16, down: bool },
}

impl Input {
    fn operation(self) -> Option<ironrdp_input::Operation> {
        use ironrdp_input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};
        Some(match self {
            Input::Move { x, y } => Operation::MouseMove(MousePosition { x, y }),
            Input::Button { button, down } => {
                let b = MouseButton::from_web_button(button)?;
                if down { Operation::MouseButtonPressed(b) } else { Operation::MouseButtonReleased(b) }
            }
            Input::Wheel { delta } => Operation::WheelRotations(WheelRotations {
                is_vertical: true,
                rotation_units: delta,
            }),
            Input::Key { scancode, down } => {
                let c = Scancode::from(scancode);
                if down { Operation::KeyPressed(c) } else { Operation::KeyReleased(c) }
            }
        })
    }
}

/// How long a read blocks before we look at the input queue. Short enough to feel
/// immediate, long enough that an idle session isn't a spin loop.
const POLL: Duration = Duration::from_millis(50);

/// The handshake is several round trips — TLS, then CredSSP, then capability
/// exchange — and `POLL` applied to any of them aborts the connection mid-negotiation.
/// It only becomes the poll interval once there is a session to poll.
const HANDSHAKE: Duration = Duration::from_secs(15);

/// A connected session, not yet pumping. Split from [`pump`] so a wrong password or
/// an unreachable host fails while the window is still waiting on the command, rather
/// than on a thread nobody is listening to yet.
pub struct Session {
    framed: ironrdp_blocking::Framed<Upgraded>,
    stage: ActiveStage,
    image: DecodedImage,
    host: String,
    clipboard: Receiver<ClipboardMessage>,
    last_seen: crate::clipboard::LastSeen,
    pub width: u16,
    pub height: u16,
}

fn config(username: String, password: String, domain: Option<String>, width: u16, height: u16) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword { username, password },
        domain,
        // Advertise both and let the server pick. Pinning either one fails against
        // half the world: a box with NLA off rejects HYBRID, and one with
        // `security_layer=tls` rejects a client that only offers CredSSP.
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize { width, height },
        bitmap: None,
        client_build: 0,
        client_name: "patchbay".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(target_os = "windows")]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        platform: MajorPlatformType::UNIX,
        // The server draws the cursor into the framebuffer for us; a hardware
        // pointer would mean compositing it on the canvas ourselves.
        enable_server_pointer: false,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        compression_type: None,
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: ironrdp::pdu::rdp::client_info::TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

type Upgraded = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// Dials the host and negotiates a session. Blocking, and quick enough to do inside
/// a command — the window wants "wrong password" as a returned error, not an event.
#[allow(clippy::too_many_arguments)]
pub fn open(
    host: &str,
    port: u16,
    username: String,
    password: String,
    domain: Option<String>,
    width: u16,
    height: u16,
) -> Result<Session, String> {
    let (to_session, clipboard) = std::sync::mpsc::channel();
    let last_seen: crate::clipboard::LastSeen =
        std::sync::Arc::new(std::sync::Mutex::new(crate::clipboard::local_text()));
    let (result, framed) = connect(
        host,
        port,
        config(username, password, domain, width, height),
        crate::clipboard::Backend::new(to_session, std::sync::Arc::clone(&last_seen)),
    )?;

    let image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        result.desktop_size.width,
        result.desktop_size.height,
    );

    let stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();

    Ok(Session {
        framed,
        stage,
        image,
        host: host.to_owned(),
        clipboard,
        last_seen,
        width: result.desktop_size.width,
        height: result.desktop_size.height,
    })
}

/// Pumps decoded rectangles to `on_tile` until the server hangs up or
/// [`Input::Close`] arrives. Blocking on purpose — it owns a thread, like `pty.rs`.
/// Free of Tauri so it can be driven from a test.
pub fn pump(session: Session, input: Receiver<Input>, on_tile: impl Fn(Tile)) -> Result<(), String> {
    let Session { mut framed, mut stage, mut image, host, clipboard, last_seen, .. } = session;
    let mut keys = ironrdp_input::Database::new();
    // No OS gives an event for "the clipboard changed", so every RDP client polls.
    // ponytail: 500ms is below noticing; a platform watcher would be the upgrade,
    // and macOS's changeCount is the only cheap one of the three.
    let mut checked = std::time::Instant::now();

    loop {
        for frame in clipboard_frames(&mut stage, &clipboard, &last_seen, &mut checked)? {
            framed.write_all(&frame).map_err(|e| format!("{host}: {e}"))?;
        }

        // Drain rather than take one: a mouse drag arrives as a burst, and one event
        // per socket timeout would make the cursor crawl.
        let mut ops = Vec::new();
        loop {
            match input.try_recv() {
                Ok(i) => ops.extend(i.operation()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if !ops.is_empty() {
            let events = keys.apply(ops);
            let outputs = stage
                .process_fastpath_input(&mut image, &events)
                .map_err(|e| e.to_string())?;
            if drain(outputs, &mut framed, &image, &host, &on_tile)? {
                return Ok(());
            }
        }

        let (action, payload) = match framed.read_pdu() {
            Ok(pdu) => pdu,
            // The read timeout is how we get back here to check the input queue;
            // an idle desktop sends nothing for minutes at a time.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => return Err(format!("{host}: {e}")),
        };

        let outputs = stage.process(&mut image, action, &payload).map_err(|e| e.to_string())?;
        if drain(outputs, &mut framed, &image, &host, &on_tile)? {
            return Ok(());
        }
    }
}

/// How often the local clipboard is compared against what we last advertised.
const CLIPBOARD_POLL: Duration = Duration::from_millis(500);

/// Turns anything the clipboard backend wants to say — plus a local copy the user
/// just made — into frames for the session to write. Kept here rather than in
/// `clipboard.rs` so every socket write stays on the thread that owns the socket.
fn clipboard_frames(
    stage: &mut ActiveStage,
    inbox: &Receiver<ClipboardMessage>,
    last_seen: &crate::clipboard::LastSeen,
    checked: &mut std::time::Instant,
) -> Result<Vec<Vec<u8>>, String> {
    let mut pending: Vec<ClipboardMessage> = inbox.try_iter().collect();

    if checked.elapsed() >= CLIPBOARD_POLL {
        *checked = std::time::Instant::now();
        let now = crate::clipboard::local_text();
        let mut seen = last_seen.lock().unwrap();
        if now != *seen {
            *seen = now;
            if seen.is_some() {
                pending.push(ClipboardMessage::SendInitiateCopy(
                    crate::clipboard::Backend::text_formats(),
                ));
            }
        }
    }
    if pending.is_empty() {
        return Ok(Vec::new());
    }

    let mut frames = Vec::new();
    for msg in pending {
        let Some(cliprdr) = stage.get_svc_processor_mut::<CliprdrClient>() else {
            // The server declined the channel, so there is no clipboard to share.
            return Ok(frames);
        };
        let messages = match msg {
            ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats),
            ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format),
            ClipboardMessage::SendFormatData(response) => cliprdr.submit_format_data(response),
            // Files aren't offered, so the remote never asks for them.
            _ => continue,
        }
        .map_err(|e| e.to_string())?;

        let frame = stage
            .process_svc_processor_messages(messages)
            .map_err(|e| e.to_string())?;
        if !frame.is_empty() {
            frames.push(frame);
        }
    }
    Ok(frames)
}

/// Copies one dirty rectangle out of the framebuffer. Only the changed region
/// crosses into the webview — a full 1280x1024 frame is 5 MB of RGBA, and a blinking
/// cursor would otherwise send all of it.
///
/// The rectangle comes off the wire, so it is clamped to the framebuffer rather than
/// trusted: a reversed or oversized region would otherwise underflow the width
/// subtraction or index past the buffer, and panic the thread the session runs on.
/// Returns `None` when nothing of it lands inside the image.
fn crop(image: &DecodedImage, region: ironrdp::pdu::geometry::InclusiveRectangle) -> Option<Tile> {
    let (iw, ih) = (image.width(), image.height());
    let left = region.left.min(iw.saturating_sub(1));
    let top = region.top.min(ih.saturating_sub(1));
    let right = region.right.min(iw.saturating_sub(1));
    let bottom = region.bottom.min(ih.saturating_sub(1));
    if iw == 0 || ih == 0 || right < left || bottom < top {
        return None;
    }

    let stride = usize::from(iw) * 4;
    let (x, y) = (usize::from(left), usize::from(top));
    // Inclusive, so a one-pixel rectangle has left == right.
    let w = usize::from(right - left) + 1;
    let h = usize::from(bottom - top) + 1;

    let data = image.data();
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let start = (y + row) * stride + x * 4;
        rgba.extend_from_slice(data.get(start..start + w * 4)?);
    }
    Some(Tile { x: left, y: top, width: w as u16, height: h as u16, rgba })
}

/// Everything the active stage can hand back, in one place — input and PDU
/// processing both produce the same outputs, and handling them in only one of the
/// two is how a repaint or a disconnect goes missing. `Ok(true)` means terminate.
fn drain(
    outputs: Vec<ActiveStageOutput>,
    framed: &mut ironrdp_blocking::Framed<Upgraded>,
    image: &DecodedImage,
    host: &str,
    on_tile: &impl Fn(Tile),
) -> Result<bool, String> {
    for out in outputs {
        match out {
            ActiveStageOutput::ResponseFrame(frame) => {
                framed.write_all(&frame).map_err(|e| format!("{host}: {e}"))?;
            }
            ActiveStageOutput::GraphicsUpdate(region) => {
                if let Some(tile) = crop(image, region) {
                    on_tile(tile);
                }
            }
            ActiveStageOutput::Terminate(_) => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

fn connect(
    host: &str,
    port: u16,
    config: connector::Config,
    clipboard: crate::clipboard::Backend,
) -> Result<(ConnectionResult, ironrdp_blocking::Framed<Upgraded>), String> {
    // A plain `connect` waits on the OS default — over a minute on a host that drops
    // the SYN rather than refusing it, which is exactly what a firewalled RDP box
    // does. The status probe and the tunnel wait already bound theirs; this was the
    // one that didn't, and it's the one someone is sitting in front of.
    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| format!("{host}:{port}: no address for that host"))?;
    let stream =
        TcpStream::connect_timeout(&addr, HANDSHAKE).map_err(|e| format!("{host}:{port}: {e}"))?;
    stream.set_read_timeout(Some(HANDSHAKE)).map_err(|e| e.to_string())?;
    let client_addr = stream.local_addr().map_err(|e| e.to_string())?;
    // Shares the underlying socket, so this is how the timeout gets shortened once
    // the stream itself is buried inside the TLS wrapper and the framing.
    let socket = stream.try_clone().map_err(|e| e.to_string())?;

    let mut framed = ironrdp_blocking::Framed::new(stream);
    let mut connector = connector::ClientConnector::new(config, client_addr)
        .with_static_channel(CliprdrClient::new(Box::new(clipboard)));
    let should_upgrade =
        ironrdp_blocking::connect_begin(&mut framed, &mut connector).map_err(|e| format!("{host}: {e}"))?;

    let (upgraded_stream, server_public_key) = tls(framed.into_inner_no_leftover(), host)?;
    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector);
    let mut upgraded_framed = ironrdp_blocking::Framed::new(upgraded_stream);

    let result = ironrdp_blocking::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut sspi::network_client::reqwest_network_client::ReqwestNetworkClient,
        host.into(),
        server_public_key,
        None,
    )
    .map_err(|e| format!("{host}: {e}"))?;

    socket.set_read_timeout(Some(POLL)).map_err(|e| e.to_string())?;
    Ok((result, upgraded_framed))
}

fn tls(stream: TcpStream, host: &str) -> Result<(Upgraded, Vec<u8>), String> {
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(verifier::AcceptAny))
        .with_no_client_auth();
    // CredSSP explicitly does not support TLS session resumption.
    config.resumption = rustls::client::Resumption::disabled();

    let name = host.to_owned().try_into().map_err(|_| format!("\"{host}\" isn't a usable server name"))?;
    let client = rustls::ClientConnection::new(std::sync::Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut tls_stream = rustls::StreamOwned::new(client, stream);
    // Without a flush the handshake hasn't moved far enough for a peer certificate.
    tls_stream.flush().map_err(|e| format!("{host}: {e}"))?;

    let cert = tls_stream
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .ok_or_else(|| format!("{host} sent no certificate"))?;
    // Before `connect_finalize`, which is where the password goes over the wire —
    // a host whose key changed must not be handed credentials.
    trust::check(host, cert)?;
    let key = public_key(cert)?;
    Ok((tls_stream, key))
}

/// Trust on first use, the way ssh does it. Verifying RDP certificates against a
/// trust store is not an option — essentially every RDP host is self-signed, and
/// mstsc just prompts — but silently accepting *any* certificate forever means a
/// swapped one goes unnoticed. So: remember the first, refuse a change.
mod trust {
    use std::path::PathBuf;

    /// Beside the config, for the same reason `known_hosts` sits beside `.ssh/config`.
    fn store() -> PathBuf {
        crate::patchbay::config_path().with_file_name("rdp_known_hosts")
    }

    fn fingerprint(cert: &[u8]) -> String {
        // FIPS-180 SHA-256, the same digest ssh prints for a host key.
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, cert);
        sha2::Digest::finalize(hasher).iter().map(|b| format!("{b:02x}")).collect()
    }

    pub(super) fn check(host: &str, cert: &[u8]) -> Result<(), String> {
        let path = store();
        let seen = std::fs::read_to_string(&path).unwrap_or_default();
        let now = fingerprint(cert);

        match seen.lines().find_map(|l| l.split_once(' ').filter(|(h, _)| *h == host)) {
            Some((_, known)) if known == now => Ok(()),
            Some((_, known)) => Err(format!(
                "{host} presented a different certificate than last time \
                 ({} instead of {}) — if the host was rebuilt, remove its line from {}",
                &now[..16.min(now.len())],
                &known[..16.min(known.len())],
                path.display()
            )),
            // First sight: remember it. A failure to write is not a reason to refuse
            // the connection, only to keep asking again next time.
            None => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                use std::io::Write as _;
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .and_then(|mut f| writeln!(f, "{host} {now}"));
                Ok(())
            }
        }
    }
}

fn public_key(cert: &[u8]) -> Result<Vec<u8>, String> {
    use x509_cert::der::Decode as _;
    x509_cert::Certificate::from_der(cert)
        .map_err(|e| e.to_string())?
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| "the server's public key is malformed".to_string())
        .map(<[u8]>::to_owned)
}

/// rustls has to return a verdict before we can see the certificate at all, so the
/// real check is [`trust`], which runs on the peer certificate straight after the
/// handshake and before any credential is sent.
mod verifier {
    use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use tokio_rustls::rustls::{pki_types, DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub(super) struct AcceptAny;

    impl ServerCertVerifier for AcceptAny {
        fn verify_server_cert(
            &self,
            _: &pki_types::CertificateDer<'_>,
            _: &[pki_types::CertificateDer<'_>],
            _: &pki_types::ServerName<'_>,
            _: &[u8],
            _: pki_types::UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            use SignatureScheme::*;
            vec![
                RSA_PKCS1_SHA1,
                ECDSA_SHA1_Legacy,
                RSA_PKCS1_SHA256,
                ECDSA_NISTP256_SHA256,
                RSA_PKCS1_SHA384,
                ECDSA_NISTP384_SHA384,
                RSA_PKCS1_SHA512,
                ECDSA_NISTP521_SHA512,
                RSA_PSS_SHA256,
                RSA_PSS_SHA384,
                RSA_PSS_SHA512,
                ED25519,
                ED448,
            ]
        }
    }
}

/// Live sessions, keyed the same way `pty::Sessions` keys terminal tabs.
#[derive(Default)]
pub struct Sessions(std::sync::Mutex<std::collections::HashMap<u32, std::sync::mpsc::Sender<Input>>>);

impl Sessions {
    /// Connects synchronously — a bad password comes back as a returned error — then
    /// leaves a thread pumping tiles into `on_tile` until it's closed. When that
    /// thread ends, `rdp-exit:<id>` carries why, so a server that hangs up leaves a
    /// dead tab rather than a frozen picture that still looks connected.
    ///
    /// Tiles go over an ipc channel rather than an event: `emit` serializes a
    /// `Vec<u8>` as a JSON array of numbers, which turns a 1 MB rectangle into
    /// several MB of text. Each message is an 8-byte header (x, y, w, h as
    /// little-endian u16) followed by raw RGBA.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        id: u32,
        host: &str,
        port: u16,
        username: String,
        password: String,
        domain: Option<String>,
        width: u16,
        height: u16,
        on_tile: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
        app: tauri::AppHandle,
    ) -> Result<Screen, String> {
        let session = open(host, port, username, password, domain, width, height)?;
        let screen = Screen { width: session.width, height: session.height };

        let (tx, rx) = std::sync::mpsc::channel();
        self.0.lock().unwrap().insert(id, tx);

        std::thread::spawn(move || {
            let ended = pump(session, rx, |t| {
                let mut msg = Vec::with_capacity(8 + t.rgba.len());
                for v in [t.x, t.y, t.width, t.height] {
                    msg.extend_from_slice(&v.to_le_bytes());
                }
                msg.extend_from_slice(&t.rgba);
                let _ = on_tile.send(tauri::ipc::InvokeResponseBody::Raw(msg));
            });
            let _ = app.emit(&format!("rdp-exit:{id}"), ended.err());
        });

        Ok(screen)
    }

    /// Silently drops input for a session that has already ended — the window can
    /// still be dispatching a mousemove when the server hangs up.
    pub fn send(&self, id: u32, input: Input) {
        if let Some(tx) = self.0.lock().unwrap().get(&id) {
            let _ = tx.send(input);
        }
    }

    /// Dropping the sender makes the pump thread's `try_recv` report a disconnect,
    /// which ends the loop and closes the socket.
    pub fn close(&self, id: u32) {
        self.0.lock().unwrap().remove(&id);
    }
}

pub type Shared = std::sync::Arc<Sessions>;

/// The desktop size the server actually gave us, which is not always what we asked
/// for — a Windows host can refuse an odd resolution and pick its own.
#[derive(serde::Serialize)]
pub struct Screen {
    pub width: u16,
    pub height: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing unit tests can't reach: whether `crop` lifts the right pixels
    /// out of the framebuffer. Needs a real host, so it's opt-in:
    ///
    /// ```sh
    /// RDP_HOST=10.0.0.26 RDP_USER=you RDP_PASS=… cargo test -- --ignored --nocapture
    /// ```
    ///
    /// Writes `rdp-frame.ppm`, which is the reassembled desktop.
    #[test]
    #[ignore = "needs a real RDP host"]
    fn tiles_reassemble_into_the_desktop() {
        let host = std::env::var("RDP_HOST").expect("RDP_HOST");
        let user = std::env::var("RDP_USER").expect("RDP_USER");
        let pass = std::env::var("RDP_PASS").expect("RDP_PASS");
        let port = std::env::var("RDP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3389);

        // Held so the closure can drop it: that is how a real session ends, so the
        // test exercises the same path `Sessions::close` uses.
        let (tx, rx) = std::sync::mpsc::channel::<Input>();
        let tx = std::sync::Mutex::new(Some(tx));
        let canvas = std::sync::Mutex::new((Vec::<u8>::new(), 0u16, 0u16));
        let tiles = std::sync::atomic::AtomicUsize::new(0);

        let session = open(&host, port, user, pass, None, 1280, 1024).expect("connect");
        {
            let mut c = canvas.lock().unwrap();
            *c = (vec![0u8; usize::from(session.width) * usize::from(session.height) * 4], session.width, session.height);
            println!("desktop {}x{}", session.width, session.height);
        }

        let out = pump(session, rx, |t| {
                let mut c = canvas.lock().unwrap();
                let (buf, w, _) = &mut *c;
                let stride = usize::from(*w) * 4;
                for row in 0..usize::from(t.height) {
                    let dst = (usize::from(t.y) + row) * stride + usize::from(t.x) * 4;
                    let src = row * usize::from(t.width) * 4;
                    buf[dst..dst + usize::from(t.width) * 4]
                        .copy_from_slice(&t.rgba[src..src + usize::from(t.width) * 4]);
                }
                // Enough of the desktop to judge; an idle session then goes quiet.
                if tiles.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 40 {
                    tx.lock().unwrap().take();
                }
        });

        let (buf, w, h) = &*canvas.lock().unwrap();
        assert!(out.is_ok(), "session failed: {out:?}");
        assert!(*w > 0 && !buf.is_empty(), "no screen size was reported");

        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.extend(buf.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write("rdp-frame.ppm", ppm).unwrap();

        let lit = buf.chunks_exact(4).filter(|p| p[0] | p[1] | p[2] != 0).count();
        println!("{} tiles, {lit} non-black pixels -> rdp-frame.ppm", tiles.load(std::sync::atomic::Ordering::SeqCst));
        assert!(lit > buf.len() / 40, "framebuffer came out essentially black — crop is wrong");
    }

    #[test]
    fn a_changed_certificate_is_refused_and_a_first_one_is_remembered() {
        let dir = std::env::temp_dir().join(format!("patchbay-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("rdp_known_hosts");
        // `check` reads $PATCHBAY_CONFIG's directory, which is how the tests point
        // every path-dependent piece of this at a scratch file.
        std::env::set_var("PATCHBAY_CONFIG", dir.join("patchbay.toml"));

        assert!(trust::check("box", b"first cert").is_ok(), "first sight is trusted");
        assert!(std::fs::read_to_string(&store).unwrap().contains("box "), "and recorded");
        assert!(trust::check("box", b"first cert").is_ok(), "the same one still is");

        let err = trust::check("box", b"a different cert").unwrap_err();
        assert!(err.contains("different certificate"), "got {err}");
        // A host we have never seen is unaffected by another one's entry.
        assert!(trust::check("other", b"whatever").is_ok());

        std::env::remove_var("PATCHBAY_CONFIG");
    }

    #[test]
    fn a_refused_connection_names_the_host_and_port() {
        // Port 1 is reserved and nothing listens on it, so this fails at TCP connect.
        let err = open("127.0.0.1", 1, "u".into(), "p".into(), None, 1024, 768)
            .err()
            .expect("nothing listens on port 1");
        assert!(err.contains("127.0.0.1:1"), "got {err}");
    }
}
