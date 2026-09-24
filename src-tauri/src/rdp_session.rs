//! Remote desktop in the window: IronRDP (pure Rust, no C dependency) decoded to a
//! framebuffer that the webview paints on a `<canvas>`. The only place patchbay
//! speaks a protocol itself. A host behind a bastion arrives through the local port
//! `rdp.rs` forwards, so this module only ever dials directly.

use ironrdp::connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationState,
};
use ironrdp::connector::{self, ConnectionResult, Credentials, Sequence as _};
use ironrdp::displaycontrol::client::DisplayControlClient;
use ironrdp::displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp::dvc::DrdynvcClient;
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::PerformanceFlags;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::CliprdrClient;
use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use tauri::Emitter as _;
use tokio_rustls::rustls;

/// A decoded rectangle. RGBA so the webview can hand it straight to `putImageData`.
pub struct Tile {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

/// What the window can send into a running session. There is no Close: dropping the
/// sender ends the session, so a socket can't be left open by forgetting to send one.
pub enum Input {
    Move {
        x: u16,
        y: u16,
    },
    Button {
        button: u8,
        down: bool,
    },
    Wheel {
        delta: i16,
    },
    /// A PC/AT scancode, translated from `KeyboardEvent.code` on the JS side.
    Key {
        scancode: u16,
        down: bool,
    },
    /// The pane is a different size; ask the desktop to follow.
    Resize {
        width: u16,
        height: u16,
        /// Percent, the window's `devicePixelRatio` times 100.
        scale: u32,
    },
}

impl Input {
    /// [`Input::Resize`] is not an input event and is taken out of the queue before
    /// this is called; `None` here would silently drop it.
    fn operation(self) -> Option<ironrdp_input::Operation> {
        use ironrdp_input::{MouseButton, MousePosition, Operation, Scancode, WheelRotations};
        Some(match self {
            Input::Move { x, y } => Operation::MouseMove(MousePosition { x, y }),
            Input::Button { button, down } => {
                let b = MouseButton::from_web_button(button)?;
                if down {
                    Operation::MouseButtonPressed(b)
                } else {
                    Operation::MouseButtonReleased(b)
                }
            }
            Input::Wheel { delta } => Operation::WheelRotations(WheelRotations {
                is_vertical: true,
                rotation_units: delta,
            }),
            Input::Key { scancode, down } => {
                let c = Scancode::from(scancode);
                if down {
                    Operation::KeyPressed(c)
                } else {
                    Operation::KeyReleased(c)
                }
            }
            Input::Resize { .. } => return None,
        })
    }
}

/// How long a read blocks before the input queue is checked.
const POLL: Duration = Duration::from_millis(50);

/// The handshake is several round trips (TLS, CredSSP, capabilities); `POLL` applied
/// to any of them aborts it mid-negotiation.
const HANDSHAKE: Duration = Duration::from_secs(15);

/// A connected session, not yet pumping. Split from [`pump`] so a wrong password
/// fails while the window is still waiting on the command.
pub struct Session {
    framed: ironrdp_blocking::Framed<Upgraded>,
    stage: ActiveStage,
    image: DecodedImage,
    host: String,
    clipboard: Receiver<ClipboardMessage>,
    last_seen: crate::clipboard::LastSeen,
    /// Builds the sequence a Server Deactivate All has to be answered with, which is
    /// how a resize takes effect. Invariant for the life of the connection.
    activation: ConnectionActivationFactory,
    pub width: u16,
    pub height: u16,
}

fn config(
    username: String,
    password: String,
    domain: Option<String>,
    width: u16,
    height: u16,
    scale: u32,
) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword { username, password },
        domain,
        // Advertise both and let the server pick: a box with NLA off rejects HYBRID,
        // one with `security_layer=tls` rejects a CredSSP-only client.
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
        // The server draws the cursor into the framebuffer; a hardware pointer would
        // have to be composited on the canvas.
        enable_server_pointer: false,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        compression_type: None,
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        // The desktop is sized in device pixels, so without this a 2x screen gets
        // everything at half size.
        desktop_scale_factor: percent(scale),
        hardware_id: None,
        license_cache: None,
        timezone_info: ironrdp::pdu::rdp::client_info::TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

type Upgraded = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// Dial the host and negotiate a session. Blocking, so "wrong password" is a returned
/// error rather than an event.
#[allow(clippy::too_many_arguments)]
pub fn open(
    host: &str,
    port: u16,
    pin: &str,
    username: String,
    password: String,
    domain: Option<String>,
    width: u16,
    height: u16,
    scale: u32,
) -> Result<Session, String> {
    let (to_session, clipboard) = std::sync::mpsc::channel();
    let last_seen = crate::clipboard::last_seen();
    let (result, framed) = connect(
        host,
        port,
        pin,
        config(username, password, domain, width, height, scale),
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
        activation: result.activation_factory,
        width: result.desktop_size.width,
        height: result.desktop_size.height,
    })
}

/// Pump decoded rectangles to `on_tile` until the server hangs up or the input sender
/// is dropped. Owns its thread, like `pty.rs`, and is free of Tauri for testing.
pub fn pump(
    session: Session,
    input: Receiver<Input>,
    on_tile: impl Fn(Tile),
    on_resize: impl Fn(u16, u16),
) -> Result<(), String> {
    let Session {
        mut framed,
        mut stage,
        mut image,
        host,
        clipboard,
        last_seen,
        activation,
        ..
    } = session;
    let mut keys = ironrdp_input::Database::new();
    // Polled: no desktop delivers a clipboard change event to a process that isn't
    // focused. The OS change counter makes each tick one integer where it has one.
    let mut poll = ClipboardPoll::default();

    loop {
        for frame in clipboard_frames(&mut stage, &clipboard, &last_seen, &mut poll)? {
            framed
                .write_all(&frame)
                .map_err(|e| format!("{host}: {e}"))?;
        }

        // Drain the queue: a mouse drag is a burst, and one event per timeout crawls.
        let mut ops = Vec::new();
        let mut resize = None;
        loop {
            match input.try_recv() {
                // Only the last size matters: dragging a window edge is a burst of them.
                Ok(Input::Resize {
                    width,
                    height,
                    scale,
                }) => resize = Some((width, height, scale)),
                Ok(i) => ops.extend(i.operation()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if let Some((width, height, scale)) = resize {
            if let Some(frame) = ask_resize(&mut stage, &image, width, height, scale) {
                framed
                    .write_all(&frame?)
                    .map_err(|e| format!("{host}: {e}"))?;
            }
        }
        if !ops.is_empty() {
            let events = keys.apply(ops);
            let outputs = stage
                .process_fastpath_input(&mut image, &events)
                .map_err(|e| e.to_string())?;
            if drain(
                outputs,
                &mut framed,
                &mut stage,
                &mut image,
                &activation,
                &host,
                &on_tile,
                &on_resize,
            )? {
                return Ok(());
            }
        }

        let (action, payload) = match framed.read_pdu() {
            Ok(pdu) => pdu,
            // The read timeout is how the input queue gets checked on an idle desktop.
            Err(e) if waiting(&e) => continue,
            Err(e) => return Err(format!("{host}: {e}")),
        };

        let outputs = stage
            .process(&mut image, action, &payload)
            .map_err(|e| e.to_string())?;
        if drain(
            outputs,
            &mut framed,
            &mut stage,
            &mut image,
            &activation,
            &host,
            &on_tile,
            &on_resize,
        )? {
            return Ok(());
        }
    }
}

/// A read that only ran out of time, rather than failing.
fn waiting(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Ask the desktop to match the pane, over the Display Control channel (MS-RDPEDISP).
/// `None` when it is already that size, or when the server never opened the channel -
/// an older host keeps the size it was opened at and the canvas scales instead.
fn ask_resize(
    stage: &mut ActiveStage,
    image: &DecodedImage,
    width: u16,
    height: u16,
    scale: u32,
) -> Option<Result<Vec<u8>, String>> {
    let (width, height) = MonitorLayoutEntry::adjust_display_size(width.into(), height.into());
    if (width, height) == (u32::from(image.width()), u32::from(image.height())) {
        return None;
    }
    Some(
        stage
            .encode_resize(width, height, Some(percent(scale)), None)?
            .map_err(|e| e.to_string()),
    )
}

/// MS-RDPEDISP's range. Out of it the resize fails to encode, which ends the session,
/// so a zoomed-out webview's 0.9 has to become 100 rather than go through.
fn percent(scale: u32) -> u32 {
    scale.clamp(100, 500)
}

/// MS-RDPBCGR 1.3.1.3: capabilities exchanged again on the same socket, which is what
/// a Server Deactivate All asks for and how a resize actually lands. Answers with the
/// size the server settled on, which is not always the one that was asked for.
fn reactivate(
    framed: &mut ironrdp_blocking::Framed<Upgraded>,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    activation: &ConnectionActivationFactory,
    host: &str,
) -> Result<(u16, u16), String> {
    let mut sequence = activation.create();
    let mut buf = ironrdp_core::WriteBuf::new();

    loop {
        if let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
        } = sequence.connection_activation_state()
        {
            stage.set_share_id(share_id);
            stage.set_enable_server_pointer(enable_server_pointer);
            // The frame acknowledgements carry the share id, so the fast path processor
            // is rebuilt rather than left answering for the share that just went away.
            stage.set_fastpath_processor(
                fast_path::ProcessorBuilder {
                    io_channel_id: activation.io_channel_id(),
                    user_channel_id: activation.user_channel_id(),
                    share_id,
                    enable_server_pointer,
                    pointer_software_rendering,
                    // `config` never advertises compression, so none was negotiated.
                    bulk_decompressor: None,
                }
                .build(),
            );
            // ponytail: the framebuffer starts blank and the server repaints after a
            // reactivation; a RefreshRectangle request if one ever doesn't.
            *image = DecodedImage::new(
                ironrdp_graphics::image_processing::PixelFormat::RgbA32,
                desktop_size.width,
                desktop_size.height,
            );
            return Ok((desktop_size.width, desktop_size.height));
        }

        buf.clear();
        let written = match sequence.next_pdu_hint() {
            Some(hint) => {
                // The socket's timeout is the input poll, far shorter than a round trip
                // here, so a wait is retried rather than taken for a dead connection.
                let deadline = std::time::Instant::now() + HANDSHAKE;
                let pdu = loop {
                    match framed.read_by_hint(hint) {
                        Ok(pdu) => break pdu,
                        Err(e) if waiting(&e) && std::time::Instant::now() < deadline => continue,
                        Err(e) if waiting(&e) => {
                            return Err(format!("\"{host}\" stopped answering mid-resize"))
                        }
                        Err(e) => return Err(format!("{host}: {e}")),
                    }
                };
                sequence.step(&pdu, &mut buf)
            }
            None => sequence.step_no_input(&mut buf),
        }
        .map_err(|e| format!("{host}: {e}"))?;

        if let Some(len) = written.size() {
            framed
                .write_all(&buf[..len])
                .map_err(|e| format!("{host}: {e}"))?;
        }
    }
}

/// How often the local clipboard is compared against what was last advertised.
const CLIPBOARD_POLL: Duration = Duration::from_millis(250);

/// When the clipboard was last looked at, and the OS change count it had then.
struct ClipboardPoll {
    checked: std::time::Instant,
    stamp: Option<u64>,
}

impl Default for ClipboardPoll {
    fn default() -> Self {
        Self {
            checked: std::time::Instant::now(),
            stamp: None,
        }
    }
}

/// Clipboard backend messages, plus a fresh local copy, as frames for the session to
/// write. Here rather than in `clipboard.rs` so socket writes stay on this thread.
fn clipboard_frames(
    stage: &mut ActiveStage,
    inbox: &Receiver<ClipboardMessage>,
    last_seen: &crate::clipboard::LastSeen,
    poll: &mut ClipboardPoll,
) -> Result<Vec<Vec<u8>>, String> {
    let mut pending: Vec<ClipboardMessage> = inbox.try_iter().collect();

    if poll.checked.elapsed() >= CLIPBOARD_POLL {
        poll.checked = std::time::Instant::now();
        // The counter is the cheap question; the text is only read once it moved. With
        // no counter the text is the comparison, as before.
        let stamp = crate::clipboard::stamp();
        let moved = stamp.is_none() || stamp != poll.stamp;
        poll.stamp = stamp;
        if moved {
            let now = crate::clipboard::local();
            let mut state = last_seen.lock().unwrap();
            if now != state.seen {
                if let Some(now) = &now {
                    pending.push(crate::clipboard::offer(now, &mut state));
                }
                state.seen = now;
            }
        }
    }
    if pending.is_empty() {
        return Ok(Vec::new());
    }

    let mut frames = Vec::new();
    for msg in pending {
        let Some(cliprdr) = stage.get_svc_processor_mut::<CliprdrClient>() else {
            // The server declined the channel.
            return Ok(frames);
        };
        let messages = match msg {
            ClipboardMessage::SendInitiateCopy(formats) => cliprdr.initiate_copy(&formats),
            ClipboardMessage::SendInitiatePaste(format) => cliprdr.initiate_paste(format),
            ClipboardMessage::SendFormatData(response) => cliprdr.submit_format_data(response),
            ClipboardMessage::SendInitiateFileCopy(files) => cliprdr.initiate_file_copy(files),
            ClipboardMessage::SendFileContentsRequest(r) => cliprdr.request_file_contents(r),
            ClipboardMessage::SendFileContentsResponse(r) => cliprdr.submit_file_contents(r),
            ClipboardMessage::Error(_) => continue,
        };
        // A clipboard the server won't take (files where it negotiated no streams) is
        // a paste that doesn't happen, not a reason to drop the desktop.
        let Ok(messages) = messages else {
            continue;
        };

        let frame = stage
            .process_svc_processor_messages(messages)
            .map_err(|e| e.to_string())?;
        if !frame.is_empty() {
            frames.push(frame);
        }
    }
    Ok(frames)
}

/// Copy one dirty rectangle out of the framebuffer, so only the changed region
/// crosses into the webview. The rectangle comes off the wire, so it is clamped
/// rather than trusted: a reversed or oversized one would panic the session thread.
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
    Some(Tile {
        x: left,
        y: top,
        width: w as u16,
        height: h as u16,
        rgba,
    })
}

/// Handle every output of the active stage in one place; input and PDU processing
/// produce the same kinds. `Ok(true)` means terminate.
#[allow(clippy::too_many_arguments)]
fn drain(
    outputs: Vec<ActiveStageOutput>,
    framed: &mut ironrdp_blocking::Framed<Upgraded>,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    activation: &ConnectionActivationFactory,
    host: &str,
    on_tile: &impl Fn(Tile),
    on_resize: &impl Fn(u16, u16),
) -> Result<bool, String> {
    for out in outputs {
        match out {
            ActiveStageOutput::ResponseFrame(frame) => {
                framed
                    .write_all(&frame)
                    .map_err(|e| format!("{host}: {e}"))?;
            }
            ActiveStageOutput::GraphicsUpdate(region) => {
                if let Some(tile) = crop(image, region) {
                    on_tile(tile);
                }
            }
            ActiveStageOutput::Terminate(_) => return Ok(true),
            // The server rebuilt the session: a resize taking effect, or the logon
            // desktop handing over. Follow it, and tell the window the new size.
            ActiveStageOutput::DeactivateAll => {
                let (width, height) = reactivate(framed, stage, image, activation, host)?;
                on_resize(width, height);
            }
            _ => {}
        }
    }
    Ok(false)
}

#[cfg(unix)]
fn with_drive(connector: connector::ClientConnector) -> connector::ClientConnector {
    connector
        .with_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(
            ironrdp_rdpsnd::client::NoopRdpsndBackend,
        )))
        .with_static_channel(
            ironrdp_rdpdr::Rdpdr::new(Box::new(crate::drive::Folder::new()), "patchbay".into())
                .with_drives(Some(vec![(1, crate::drive::DRIVE.into())])),
        )
}

// ponytail: the backend is unix-only, so a Windows copy shares no folder; its own
// mstsc handoff does.
#[cfg(not(unix))]
fn with_drive(connector: connector::ClientConnector) -> connector::ClientConnector {
    connector
}

fn connect(
    host: &str,
    port: u16,
    pin: &str,
    config: connector::Config,
    clipboard: crate::clipboard::Backend,
) -> Result<(ConnectionResult, ironrdp_blocking::Framed<Upgraded>), String> {
    // Bounded: a firewalled host drops the SYN, and the OS default is over a minute.
    let addr = format!("{host}:{port}")
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| format!("{host}:{port}: no address for that host"))?;
    let stream =
        TcpStream::connect_timeout(&addr, HANDSHAKE).map_err(|e| format!("{host}:{port}: {e}"))?;
    stream
        .set_read_timeout(Some(HANDSHAKE))
        .map_err(|e| e.to_string())?;
    let client_addr = stream.local_addr().map_err(|e| e.to_string())?;
    // Shares the socket, so the timeout can be shortened once the stream is wrapped.
    let socket = stream.try_clone().map_err(|e| e.to_string())?;

    let mut framed = ironrdp_blocking::Framed::new(stream);
    let connector = connector::ClientConnector::new(config, client_addr)
        .with_static_channel(CliprdrClient::new(Box::new(clipboard)))
        // The Display Control channel is the only way the desktop follows the window;
        // nothing is sent when the capabilities arrive, the first resize does that.
        .with_static_channel(
            DrdynvcClient::new()
                .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new()))),
        );
    let mut connector = with_drive(connector);
    let should_upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector)
        .map_err(|e| format!("{host}: {e}"))?;

    let (upgraded_stream, server_public_key) = tls(framed.into_inner_no_leftover(), host, pin)?;
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

    socket
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    Ok((result, upgraded_framed))
}

/// `pin` is the device's own `host:port`, which the certificate is remembered under:
/// `host` is `127.0.0.1` for every device behind a bastion.
fn tls(stream: TcpStream, host: &str, pin: &str) -> Result<(Upgraded, Vec<u8>), String> {
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(verifier::AcceptAny))
        .with_no_client_auth();
    // CredSSP explicitly does not support TLS session resumption.
    config.resumption = rustls::client::Resumption::disabled();

    let name = host
        .to_owned()
        .try_into()
        .map_err(|_| format!("\"{host}\" isn't a usable server name"))?;
    let client = rustls::ClientConnection::new(std::sync::Arc::new(config), name)
        .map_err(|e| e.to_string())?;
    let mut tls_stream = rustls::StreamOwned::new(client, stream);
    // Without a flush the handshake hasn't moved far enough for a peer certificate.
    tls_stream.flush().map_err(|e| format!("{host}: {e}"))?;

    let cert = tls_stream
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .ok_or_else(|| format!("\"{host}\" sent no certificate"))?;
    // Before `connect_finalize`, where the password goes over the wire.
    trust::check(pin, cert)?;
    let key = public_key(cert)?;
    Ok((tls_stream, key))
}

/// Trust on first use, as ssh does it: nearly every RDP host is self-signed, but a
/// swapped certificate must not go unnoticed. Remember the first, refuse a change.
///
/// Keyed by the device's `host:port`. A line with a bare host is from before the port
/// was part of it, and still answers for that host so an upgrade doesn't re-trust
/// everything unseen.
pub mod trust {
    use std::path::{Path, PathBuf};

    /// Beside the config: this machine's answer, not part of the list.
    fn store() -> PathBuf {
        crate::patchbay::config_path().with_file_name("rdp_known_hosts")
    }

    fn fingerprint(cert: &[u8]) -> String {
        // SHA-256, the digest ssh prints for a host key.
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, cert);
        sha2::Digest::finalize(hasher)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn is_mine(line_key: &str, pin: &str) -> bool {
        line_key == pin
            || pin
                .rsplit_once(':')
                .is_some_and(|(host, _)| line_key == host)
    }

    pub(super) fn check(pin: &str, cert: &[u8]) -> Result<(), String> {
        check_at(&store(), pin, cert)
    }

    /// The store is one `pin fingerprint` per line, and a pin comes from a host in a
    /// file someone else may have written: a space or a newline in it writes a line
    /// of its own, trusting a certificate for some other device.
    fn usable(pin: &str) -> Result<(), String> {
        match pin.chars().any(|c| c.is_whitespace() || c.is_control()) {
            true => Err(format!("\"{}\" isn't a usable host", pin.escape_debug())),
            false => Ok(()),
        }
    }

    pub(super) fn check_at(path: &Path, pin: &str, cert: &[u8]) -> Result<(), String> {
        usable(pin)?;
        let seen = std::fs::read_to_string(path).unwrap_or_default();
        let now = fingerprint(cert);
        // The exact key first: a legacy bare-host line must not outrank a newer answer.
        let known = seen
            .lines()
            .filter_map(|l| l.split_once(' '))
            .filter(|(k, _)| is_mine(k, pin))
            .max_by_key(|(k, _)| *k == pin)
            .map(|(_, fp)| fp);

        match known {
            Some(known) if known == now => Ok(()),
            // The full new fingerprint is what the window offers to trust, and the text
            // it is read back out of.
            Some(known) => Err(format!(
                "\"{pin}\" presented a different certificate than last time - \
                 SHA-256 {now}, where it was {}",
                &known[..16.min(known.len())],
            )),
            // First sight: remember it. A failed write only means asking again next time.
            None => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                use std::io::Write as _;
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .and_then(|mut f| writeln!(f, "{pin} {now}"));
                Ok(())
            }
        }
    }

    /// "Trust the new one": the fingerprint the refusal named, and no other, replaces
    /// what was remembered. Taken from the refusal rather than fetched again, so what is
    /// trusted is exactly what the user was shown.
    pub fn accept(pin: &str, fingerprint: &str) -> Result<(), String> {
        accept_at(&store(), pin, fingerprint)
    }

    pub(super) fn accept_at(path: &Path, pin: &str, fingerprint: &str) -> Result<(), String> {
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("\"{fingerprint}\" isn't a SHA-256 fingerprint"));
        }
        usable(pin)?;
        let seen = std::fs::read_to_string(path).unwrap_or_default();
        // Only this pin's own line: a bare-host line may be another port's answer, and
        // the exact line outranks it on the way back in anyway.
        let mut out: String = seen
            .lines()
            .filter(|l| !l.split_once(' ').is_some_and(|(k, _)| k == pin))
            .map(|l| format!("{l}\n"))
            .collect();
        out.push_str(&format!("{pin} {}\n", fingerprint.to_ascii_lowercase()));
        // Temp and rename: a truncated store re-trusts every host on first sight.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, out)
            .and_then(|_| std::fs::rename(&tmp, path))
            .map_err(|e| format!("{}: {e}", path.display()))
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

/// Accepts any certificate so the handshake completes; the real check is [`trust`],
/// run on the peer certificate before any credential is sent. `commands/web.rs`
/// uses it the same way, to fetch a certificate for the user to look at.
pub(crate) mod verifier {
    use tokio_rustls::rustls::client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    };
    use tokio_rustls::rustls::{pki_types, DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub(crate) struct AcceptAny;

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

/// How long tiles are gathered before they cross to the window: one frame at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

/// `x` and `y` of a record that carries the desktop's new size rather than pixels. No
/// tile starts there: the desktop is capped at 8192.
const RESIZED: u16 = u16::MAX;

/// Live sessions, keyed the same way `pty::Sessions` keys terminal tabs.
#[derive(Default)]
pub struct Sessions(
    std::sync::Mutex<std::collections::HashMap<u32, std::sync::mpsc::Sender<Input>>>,
);

impl Sessions {
    /// Connect synchronously, then pump tiles into `on_tile` from a thread until
    /// closed; `rdp-exit:<id>` carries why it ended. Tiles go over an ipc channel, not
    /// an event, because `emit` would serialize the bytes as a JSON array. Each
    /// message is a run of records, an 8-byte header (x, y, w, h as little-endian u16)
    /// then raw RGBA; one at `RESIZED` has no pixels and is the desktop's new size.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        id: u32,
        host: &str,
        port: u16,
        pin: &str,
        username: String,
        password: String,
        domain: Option<String>,
        width: u16,
        height: u16,
        scale: u32,
        on_tile: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
        app: tauri::AppHandle,
    ) -> Result<Screen, String> {
        let session = open(
            host, port, pin, username, password, domain, width, height, scale,
        )?;
        let screen = Screen {
            width: session.width,
            height: session.height,
        };

        let (tx, rx) = std::sync::mpsc::channel();
        self.0.lock().unwrap().insert(id, tx);

        // Tiles are batched into one message per frame: each send is an eval and, past
        // 1 KB, a fetch round trip on top, and a busy desktop is hundreds of small tiles.
        let (batch, batched) = std::sync::mpsc::channel::<Vec<u8>>();
        let sender = std::thread::spawn(move || {
            while let Ok(mut msg) = batched.recv() {
                let until = std::time::Instant::now() + FRAME;
                while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
                    match batched.recv_timeout(left) {
                        Ok(more) => msg.extend_from_slice(&more),
                        Err(_) => break,
                    }
                }
                let _ = on_tile.send(tauri::ipc::InvokeResponseBody::Raw(msg));
            }
        });

        std::thread::spawn(move || {
            let send = |head: [u16; 4], rgba: &[u8]| {
                let mut msg = Vec::with_capacity(8 + rgba.len());
                for v in head {
                    msg.extend_from_slice(&v.to_le_bytes());
                }
                msg.extend_from_slice(rgba);
                let _ = batch.send(msg);
            };
            let ended = pump(
                session,
                rx,
                |t| send([t.x, t.y, t.width, t.height], &t.rgba),
                // Down the same channel as the tiles, so it can't overtake the ones
                // painted before the desktop changed size.
                |width, height| send([RESIZED, RESIZED, width, height], &[]),
            );
            // The last batch lands before the exit does, or the final frame is lost.
            drop(batch);
            let _ = sender.join();
            let _ = app.emit(&format!("rdp-exit:{id}"), ended.err());
        });

        Ok(screen)
    }

    /// Input for a session that has already ended is dropped, not an error.
    pub fn send(&self, id: u32, input: Input) {
        if let Some(tx) = self.0.lock().unwrap().get(&id) {
            let _ = tx.send(input);
        }
    }

    /// Dropping the sender ends the pump loop and closes the socket.
    pub fn close(&self, id: u32) {
        self.0.lock().unwrap().remove(&id);
    }
}

pub type Shared = std::sync::Arc<Sessions>;

/// The desktop size the server gave, which is not always the one asked for.
#[derive(serde::Serialize)]
pub struct Screen {
    pub width: u16,
    pub height: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `crop` lifts the right pixels. Needs a real host, writes `rdp-frame.ppm`:
    /// `RDP_HOST=10.0.0.5 RDP_USER=you RDP_PASS=… cargo test -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a real RDP host"]
    fn tiles_reassemble_into_the_desktop() {
        let host = std::env::var("RDP_HOST").expect("RDP_HOST");
        let user = std::env::var("RDP_USER").expect("RDP_USER");
        let pass = std::env::var("RDP_PASS").expect("RDP_PASS");
        let port = std::env::var("RDP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3389);

        // Held so the closure can drop it, the same way `Sessions::close` ends one.
        let (tx, rx) = std::sync::mpsc::channel::<Input>();
        let tx = std::sync::Mutex::new(Some(tx));
        let canvas = std::sync::Mutex::new((Vec::<u8>::new(), 0u16, 0u16));
        let tiles = std::sync::atomic::AtomicUsize::new(0);

        let pin = format!("{host}:{port}");
        let session = open(&host, port, &pin, user, pass, None, 1280, 1024, 100).expect("connect");
        {
            let mut c = canvas.lock().unwrap();
            *c = (
                vec![0u8; usize::from(session.width) * usize::from(session.height) * 4],
                session.width,
                session.height,
            );
            println!("desktop {}x{}", session.width, session.height);
        }

        let out = pump(
            session,
            rx,
            |t| {
                let mut c = canvas.lock().unwrap();
                let (buf, w, _) = &mut *c;
                let stride = usize::from(*w) * 4;
                for row in 0..usize::from(t.height) {
                    let dst = (usize::from(t.y) + row) * stride + usize::from(t.x) * 4;
                    let src = row * usize::from(t.width) * 4;
                    buf[dst..dst + usize::from(t.width) * 4]
                        .copy_from_slice(&t.rgba[src..src + usize::from(t.width) * 4]);
                }
                // Enough of the desktop to judge.
                if tiles.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 40 {
                    tx.lock().unwrap().take();
                }
            },
            |_, _| {},
        );

        let (buf, w, h) = &*canvas.lock().unwrap();
        assert!(out.is_ok(), "session failed: {out:?}");
        assert!(*w > 0 && !buf.is_empty(), "no screen size was reported");

        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        let pixels = buf.as_chunks::<4>().0;
        ppm.extend(pixels.iter().flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write("rdp-frame.ppm", ppm).unwrap();

        let lit = pixels.iter().filter(|p| p[0] | p[1] | p[2] != 0).count();
        println!(
            "{} tiles, {lit} non-black pixels -> rdp-frame.ppm",
            tiles.load(std::sync::atomic::Ordering::SeqCst)
        );
        assert!(
            lit > buf.len() / 40,
            "framebuffer came out essentially black - crop is wrong"
        );
    }

    #[test]
    fn a_changed_certificate_is_refused_and_a_first_one_is_remembered() {
        let dir = std::env::temp_dir().join(format!("patchbay-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("rdp_known_hosts");
        let check = |pin: &str, cert: &[u8]| trust::check_at(&store, pin, cert);

        assert!(
            check("box:3389", b"first cert").is_ok(),
            "first sight is trusted"
        );
        assert!(
            std::fs::read_to_string(&store)
                .unwrap()
                .contains("box:3389 "),
            "and recorded"
        );
        assert!(
            check("box:3389", b"first cert").is_ok(),
            "the same one still is"
        );

        let err = check("box:3389", b"a different cert").unwrap_err();
        assert!(err.contains("different certificate"), "got {err}");
        // Another host's entry does not affect a new one, and neither does another port
        // on the same address - two machines behind one forwarded IP.
        assert!(check("other:3389", b"whatever").is_ok());
        assert!(check("box:3390", b"a different cert").is_ok());

        // The refusal carries the whole fingerprint, and trusting it is what lets the
        // new certificate in - that one, for that device, and nothing else.
        let fp = err.split("SHA-256 ").nth(1).unwrap()[..64].to_string();
        assert!(trust::accept_at(&store, "box:3389", "not hex").is_err());
        trust::accept_at(&store, "box:3389", &fp).unwrap();
        assert!(check("box:3389", b"a different cert").is_ok());
        assert!(check("box:3389", b"first cert").is_err());
        assert!(check("box:3390", b"a different cert").is_ok());
        assert!(check("other:3389", b"whatever").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_from_before_ports_still_answers_for_its_host() {
        let dir = std::env::temp_dir().join(format!("patchbay-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("rdp_known_hosts");
        trust::check_at(&store, "box:3389", b"old").unwrap();
        // Rewrite as an old store would have it: the bare host.
        let line = std::fs::read_to_string(&store)
            .unwrap()
            .replace("box:3389", "box");
        std::fs::write(&store, &line).unwrap();

        assert!(trust::check_at(&store, "box:3389", b"old").is_ok());
        assert!(trust::check_at(&store, "box:3389", b"swapped").is_err());
        // Accepting outranks the legacy line without deleting it: that line may be the
        // answer for another port on the same address.
        let err = trust::check_at(&store, "box:3389", b"swapped").unwrap_err();
        let fp = err.split("SHA-256 ").nth(1).unwrap()[..64].to_string();
        trust::accept_at(&store, "box:3389", &fp).unwrap();
        assert!(trust::check_at(&store, "box:3389", b"swapped").is_ok());
        assert!(trust::check_at(&store, "box:3390", b"old").is_ok());
        assert!(trust::check_at(&store, "box:3390", b"swapped").is_err());

        // A host from someone else's file can't write a line of its own.
        for smuggled in ["evil\nbox:3389", "evil box:3389", "evil\r"] {
            assert!(
                trust::check_at(&store, smuggled, b"x").is_err(),
                "{smuggled:?}"
            );
            assert!(
                trust::accept_at(&store, smuggled, &fp).is_err(),
                "{smuggled:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_connection_names_the_host_and_port() {
        // Nothing listens on port 1, so this fails at TCP connect.
        let err = open(
            "127.0.0.1",
            1,
            "127.0.0.1:1",
            "u".into(),
            "p".into(),
            None,
            1024,
            768,
            100,
        )
        .err()
        .expect("nothing listens on port 1");
        assert!(err.contains("127.0.0.1:1"), "got {err}");
    }
}
