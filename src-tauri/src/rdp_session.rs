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
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::PerformanceFlags;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::CliprdrClient;
use ironrdp_egfx::client::GraphicsPipelineClient;
use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use tauri::Emitter as _;
use tokio_rustls::rustls;

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
    /// The window painted the last frame, so the next may go.
    Painted,
    /// The pane is a different size; ask the desktop to follow.
    Resize {
        width: u16,
        height: u16,
        /// Percent, the window's `devicePixelRatio` times 100.
        scale: u32,
    },
}

impl Input {
    /// [`Input::Resize`] and [`Input::Painted`] are not input events and are taken out
    /// of the queue before this is called; `None` here would silently drop them.
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
            Input::Resize { .. } | Input::Painted => return None,
        })
    }
}

/// How long a read blocks before the input queue is checked, and a frame the window is
/// ready for goes out. Half a frame: a quiet desktop otherwise holds a keystroke, or
/// the last paint of a burst, for the whole wait.
const POLL: Duration = Duration::from_millis(8);

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
    /// Graphics pipelines alive, see [`Gfx`].
    pipelines: Pipelines,
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
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize { width, height },
        monitor_layout: None,
        connection_type: ironrdp::pdu::gcc::ConnectionType::Lan,
        remote_application_mode: false,
        rail_support_level: ironrdp::pdu::rdp::capability_sets::RailSupportLevel::empty(),
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
        enable_audio_capture: false,
        compression_type: None,
        pointer_software_rendering: true,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: true,
        // A wallpaper is a photo, and the bitmap updates a Windows host sends outside
        // the graphics pipeline compress one badly: closing a maximised window revealed
        // it as hundreds of pieces over a second. What mstsc drops below a LAN link.
        performance_flags: PerformanceFlags::default() | PerformanceFlags::DISABLE_WALLPAPER,
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
    let pipelines = Pipelines::default();
    let (result, framed) = connect(
        host,
        port,
        pin,
        config(username, password, domain, width, height, scale),
        crate::clipboard::Backend::new(to_session, std::sync::Arc::clone(&last_seen)),
        &pipelines,
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
        pipelines,
        width: result.desktop_size.width,
        height: result.desktop_size.height,
    })
}

/// Pump frames to `on_frame` until the server hangs up or the input sender is dropped.
/// Owns its thread, like `pty.rs`, and is free of Tauri for testing. A frame is a run
/// of records, an 8-byte header (x, y, w, h as little-endian u16) then raw RGBA; one at
/// `RESIZED` has no pixels and is the desktop's new size.
pub fn pump(
    session: Session,
    input: Receiver<Input>,
    on_frame: impl Fn(Vec<u8>),
) -> Result<(), String> {
    let Session {
        mut framed,
        mut stage,
        mut image,
        host,
        clipboard,
        last_seen,
        activation,
        pipelines,
        ..
    } = session;
    let mut keys = ironrdp_input::Database::new();
    // Polled: no desktop delivers a clipboard change event to a process that isn't
    // focused. The OS change counter makes each tick one integer where it has one.
    let mut poll = ClipboardPoll::default();
    let mut pending = Pending {
        size: (image.width(), image.height()),
        ..Pending::default()
    };
    // Kept across passes until it is sent: the Display Control channel opens a moment
    // after the desktop appears, and a size asked for before then was lost for good.
    let mut resize = None;

    loop {
        for frame in clipboard_frames(&mut stage, &clipboard, &last_seen, &mut poll)? {
            framed
                .write_all(&frame)
                .map_err(|e| format!("{host}: {e}"))?;
        }

        // Drain the queue: a mouse drag is a burst, and one event per timeout crawls.
        let mut ops = Vec::new();
        loop {
            match input.try_recv() {
                // Only the last size matters: dragging a window edge is a burst of them.
                Ok(Input::Resize {
                    width,
                    height,
                    scale,
                }) => resize = Some((width, height, scale)),
                Ok(Input::Painted) => pending.painted = true,
                Ok(i) => ops.extend(i.operation()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if let Some((width, height, scale)) = resize {
            let (width, height) =
                MonitorLayoutEntry::adjust_display_size(width.into(), height.into());
            if (width, height) == (u32::from(image.width()), u32::from(image.height())) {
                resize = None;
            // `None` until the channel is open, and forever on a host without one, where
            // the canvas letterboxes instead.
            } else if let Some(frame) =
                stage.encode_resize(width, height, Some(percent(scale)), None)
            {
                framed
                    .write_all(&frame.map_err(|e| e.to_string())?)
                    .map_err(|e| format!("{host}: {e}"))?;
                resize = None;
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
                &mut pending,
            )? {
                return Ok(());
            }
        }

        let quiet = match framed.read_pdu() {
            Ok((action, payload)) => {
                let outputs = stage
                    .process(&mut image, action, &payload, framed.last_read_at())
                    .map_err(|e| e.to_string())?;
                if drain(
                    outputs,
                    &mut framed,
                    &mut stage,
                    &mut image,
                    &activation,
                    &host,
                    &mut pending,
                )? {
                    return Ok(());
                }
                false
            }
            // The read timeout is how the input queue gets checked on an idle desktop.
            Err(e) if waiting(&e) => true,
            Err(e) => return Err(format!("{host}: {e}")),
        };
        if pipelines.load(std::sync::atomic::Ordering::Relaxed) > MAX_PIPELINES {
            return Err(format!(
                "\"{host}\" opened its graphics channel more times than a desktop needs"
            ));
        }

        if let Some(frame) = pending.take(&image, quiet) {
            on_frame(frame);
        }
    }
}

/// How long a frame is gathered before it crosses to the window: one at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

/// The server sends one screen's change as a burst of updates and then goes quiet, so
/// a frame goes when the socket does - sent mid-burst, a page loading paints in bands
/// from the top down. This is the longest a steady stream holds one back.
const LATE: Duration = Duration::from_millis(50);

/// A frame the window never said it painted (a reload, a thrown paint) is given up on
/// after this, so a lost acknowledgement slows the picture rather than freezing it.
const STALL: Duration = Duration::from_secs(1);

/// `x` and `y` of a record that carries the desktop's new size rather than pixels. No
/// tile starts there: the desktop is capped at 8192.
const RESIZED: u16 = u16::MAX;

/// What changed since the last frame went to the window. Regions, not pixels: they are
/// cut from the framebuffer when the frame goes, so a region drawn ten times while the
/// window was busy crosses once, as it is now. Queueing the pixels instead made a
/// window that paints slower than the server draws fall further behind every second.
struct Pending {
    dirty: Vec<InclusiveRectangle>,
    resized: Option<(u16, u16)>,
    /// The desktop size the window was last told, so a change it hasn't heard of goes
    /// ahead of the pixels drawn at the new size.
    size: (u16, u16),
    /// When the oldest change still waiting was made.
    since: Option<std::time::Instant>,
    /// The window acknowledged the last frame; at most one is ever in flight.
    painted: bool,
    sent: std::time::Instant,
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            dirty: Vec::new(),
            resized: None,
            size: (0, 0),
            since: None,
            painted: true,
            sent: std::time::Instant::now(),
        }
    }
}

fn area(r: &InclusiveRectangle) -> u32 {
    (u32::from(r.right.saturating_sub(r.left)) + 1)
        * (u32::from(r.bottom.saturating_sub(r.top)) + 1)
}

fn contains(a: &InclusiveRectangle, b: &InclusiveRectangle) -> bool {
    a.left <= b.left && a.top <= b.top && a.right >= b.right && a.bottom >= b.bottom
}

impl Pending {
    /// Kept as a short list rather than one bounding box, which a clock in one corner
    /// and a cursor in the other would turn into the whole screen. Two regions become
    /// one when that costs no more pixels than sending both.
    fn mark(&mut self, mut r: InclusiveRectangle) {
        self.since.get_or_insert_with(std::time::Instant::now);
        // ponytail: a pass per region over a list of a few dozen; a real region set if a
        // desktop ever dirties hundreds of disjoint spots per frame.
        let mut i = 0;
        while i < self.dirty.len() {
            let d = &self.dirty[i];
            if contains(d, &r) {
                return;
            }
            let joined = InclusiveRectangle {
                left: d.left.min(r.left),
                top: d.top.min(r.top),
                right: d.right.max(r.right),
                bottom: d.bottom.max(r.bottom),
            };
            if area(&joined) <= area(d) + area(&r) {
                // The union may now swallow regions already passed, so start over.
                self.dirty.swap_remove(i);
                r = joined;
                i = 0;
            } else {
                i += 1;
            }
        }
        self.dirty.push(r);
    }

    /// A new desktop: whatever was dirty belonged to the old framebuffer.
    fn resize(&mut self, width: u16, height: u16) {
        self.size = (width, height);
        self.dirty.clear();
        self.resized = Some((width, height));
        self.since.get_or_insert_with(std::time::Instant::now);
    }

    /// The framebuffer is `width` by `height` now. The graphics pipeline resizes it in
    /// place (a Windows host answers a resize with ResetGraphics, not a reactivation), so
    /// the size is checked with every update rather than only when the session rebuilds.
    fn follow(&mut self, width: u16, height: u16) {
        if self.size != (width, height) {
            self.resize(width, height);
        }
    }

    /// The next frame, once the server has finished drawing it (`quiet`) or it has
    /// waited long enough, and the window has painted the last.
    fn take(&mut self, image: &DecodedImage, quiet: bool) -> Option<Vec<u8>> {
        let waited = self.since?.elapsed();
        let sent = self.sent.elapsed();
        if !(quiet || waited >= LATE) || sent < FRAME || !(self.painted || sent >= STALL) {
            return None;
        }
        self.since = None;
        let mut frame = Vec::new();
        if let Some((width, height)) = self.resized.take() {
            head(&mut frame, [RESIZED, RESIZED, width, height]);
        }
        for region in self.dirty.drain(..) {
            crop(image, region, &mut frame);
        }
        self.painted = false;
        self.sent = std::time::Instant::now();
        Some(frame)
    }
}

fn head(frame: &mut Vec<u8>, head: [u16; 4]) {
    for v in head {
        frame.extend_from_slice(&v.to_le_bytes());
    }
}

/// A read that only ran out of time, rather than failing.
fn waiting(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
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
            static_channel_chunk_size,
            ..
        } = sequence.connection_activation_state()
        {
            // The frame acknowledgements carry the share id, so the processors are rebuilt
            // rather than left answering for the share that just went away.
            if !stage.reactivate(
                activation.io_channel_id(),
                activation.user_channel_id(),
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ) {
                return Err(format!(
                    "{host}: the server asked for an invalid channel chunk size"
                ));
            }
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
                sequence.step(&pdu, framed.last_read_at(), &mut buf)
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

/// Append one dirty rectangle of the framebuffer to `frame` as a record, so only the
/// changed region crosses into the webview. The rectangle comes off the wire, so it is
/// clamped rather than trusted: a reversed or oversized one would panic the session
/// thread.
fn crop(image: &DecodedImage, region: InclusiveRectangle, frame: &mut Vec<u8>) {
    let (iw, ih) = (image.width(), image.height());
    let left = region.left.min(iw.saturating_sub(1));
    let top = region.top.min(ih.saturating_sub(1));
    let right = region.right.min(iw.saturating_sub(1));
    let bottom = region.bottom.min(ih.saturating_sub(1));
    if iw == 0 || ih == 0 || right < left || bottom < top {
        return;
    }

    let stride = usize::from(iw) * 4;
    let (x, y) = (usize::from(left), usize::from(top));
    // Inclusive, so a one-pixel rectangle has left == right.
    let w = usize::from(right - left) + 1;
    let h = usize::from(bottom - top) + 1;

    // Clamped above, so every row is inside the framebuffer.
    let data = image.data();
    head(frame, [left, top, w as u16, h as u16]);
    frame.reserve(w * h * 4);
    for row in 0..h {
        let start = (y + row) * stride + x * 4;
        frame.extend_from_slice(&data[start..start + w * 4]);
    }
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
    pending: &mut Pending,
) -> Result<bool, String> {
    for out in outputs {
        match out {
            ActiveStageOutput::ResponseFrame(frame) => {
                framed
                    .write_all(&frame)
                    .map_err(|e| format!("{host}: {e}"))?;
            }
            ActiveStageOutput::GraphicsUpdate(region) => {
                pending.follow(image.width(), image.height());
                pending.mark(region);
            }
            ActiveStageOutput::Terminate(_) => return Ok(true),
            // The server rebuilt the session: a resize taking effect, or the logon
            // desktop handing over. Follow it, and tell the window the new size.
            ActiveStageOutput::DeactivateAll => {
                let (width, height) = reactivate(framed, stage, image, activation, host)?;
                pending.resize(width, height);
            }
            _ => {}
        }
    }
    Ok(false)
}

fn connect(
    host: &str,
    port: u16,
    pin: &str,
    config: connector::Config,
    clipboard: crate::clipboard::Backend,
    pipelines: &Pipelines,
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
    let mut connector = connector::ClientConnector::new(config, client_addr)
        .with_static_channel(CliprdrClient::new(Box::new(clipboard)))
        // The Display Control channel is the only way the desktop follows the window;
        // nothing is sent when the capabilities arrive, the first resize does that.
        //
        // Both are factories rather than one processor each: a server that resizes by
        // deactivating the session (xrdp) opens them again afterwards, and a processor
        // that was handed over once would answer "no listener" and leave the desktop black.
        .with_static_channel(
            DrdynvcClient::new()
                .with_dynamic_channel_factory(ironrdp::displaycontrol::CHANNEL_NAME, || {
                    DisplayControlClient::new(|_| Ok(Vec::new()))
                })
                // The graphics pipeline: what Windows sends a client that has one, and
                // its codecs are the ones made for a desktop. The session drains its
                // compositor into the same framebuffer, so nothing past here changes.
                .with_dynamic_channel_factory(ironrdp_egfx::CHANNEL_NAME, {
                    let pipelines = std::sync::Arc::clone(pipelines);
                    move || {
                        let gfx = Gfx::new(&pipelines);
                        GraphicsPipelineClient::new(Box::new(gfx), crate::h264::decoder())
                    }
                }),
        );
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
    .and_then(|outcome| outcome.into_connected())
    .map_err(|e| format!("{host}: {e}"))?;

    socket
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    Ok((result, upgraded_framed))
}

/// The compositor inside [`GraphicsPipelineClient`] keeps the surfaces and the session
/// drains it, so none of the per-command callbacks are needed. What this does is count:
/// the channel is a factory, so a server can open it as often as it likes, each one with
/// a compositor allowed hundreds of megabytes, and the handler is dropped with its
/// pipeline, which makes it the place to keep the tally.
struct Gfx(Pipelines);

type Pipelines = std::sync::Arc<std::sync::atomic::AtomicUsize>;

/// A real server has one pipeline open, and a reopen briefly two; more is a server
/// asking for memory, and the session ends rather than give it.
const MAX_PIPELINES: usize = 2;

impl Gfx {
    fn new(pipelines: &Pipelines) -> Self {
        pipelines.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(std::sync::Arc::clone(pipelines))
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl ironrdp_egfx::client::GraphicsPipelineHandler for Gfx {}

/// Matched by the window, which hands the device to the desktop's own RDP client.
const OLD_TLS: &str = "the server only offers older encryption than the built-in session speaks";

/// A handshake the server walked out of. rustls offers only ECDHE with an AEAD cipher,
/// and Windows Server 2008 R2 has none of those: Schannel resets the socket rather than
/// sending an alert. mstsc and FreeRDP still speak its CBC and DHE suites, so the
/// handoff gets in where this can't.
fn too_old(e: &std::io::Error) -> bool {
    use rustls::{AlertDescription as A, Error as R};
    match e.get_ref().and_then(|i| i.downcast_ref::<R>()) {
        Some(R::AlertReceived(
            A::HandshakeFailure | A::ProtocolVersion | A::InsufficientSecurity,
        ))
        | Some(R::PeerIncompatible(_)) => true,
        Some(_) => false,
        None => matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
        ),
    }
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
    tls_stream.flush().map_err(|e| {
        if too_old(&e) {
            format!("{pin}: {OLD_TLS}")
        } else {
            format!("{host}: {e}")
        }
    })?;

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

/// Live sessions, keyed the same way `pty::Sessions` keys terminal tabs.
#[derive(Default)]
pub struct Sessions(
    std::sync::Mutex<std::collections::HashMap<u32, std::sync::mpsc::Sender<Input>>>,
);

impl Sessions {
    /// Connect synchronously, then pump frames into `on_tile` from a thread until
    /// closed; `rdp-exit:<id>` carries why it ended. Frames go over an ipc channel, not
    /// an event, because `emit` would serialize the bytes as a JSON array; [`pump`] has
    /// their layout.
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

        std::thread::spawn(move || {
            let ended = pump(session, rx, |frame| {
                let _ = on_tile.send(tauri::ipc::InvokeResponseBody::Raw(frame));
            });
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

    #[test]
    fn a_walked_out_handshake_is_old_encryption() {
        use std::io::{Error, ErrorKind};
        let alert = |a| Error::new(ErrorKind::InvalidData, rustls::Error::AlertReceived(a));
        assert!(too_old(&Error::from(ErrorKind::ConnectionReset)));
        assert!(too_old(&alert(rustls::AlertDescription::HandshakeFailure)));
        assert!(!too_old(&alert(rustls::AlertDescription::BadCertificate)));
        assert!(!too_old(&Error::from(ErrorKind::TimedOut)));
    }

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

        let out = pump(session, rx, |frame| {
            let mut c = canvas.lock().unwrap();
            let (buf, w, _) = &mut *c;
            let stride = usize::from(*w) * 4;
            let mut at = 0;
            while at < frame.len() {
                let [x, y, tw, th] = [0, 2, 4, 6]
                    .map(|o| usize::from(u16::from_le_bytes([frame[at + o], frame[at + o + 1]])));
                at += 8;
                assert_ne!(x, usize::from(RESIZED), "no resize was asked for");
                for row in 0..th {
                    let dst = (y + row) * stride + x * 4;
                    buf[dst..dst + tw * 4].copy_from_slice(&frame[at..at + tw * 4]);
                    at += tw * 4;
                }
            }
            // Nothing acknowledges here, so frames come a `STALL` apart. Enough to judge.
            if tiles.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 3 {
                tx.lock().unwrap().take();
            }
        });

        let (buf, w, h) = &*canvas.lock().unwrap();
        assert!(out.is_ok(), "session failed: {out:?}");
        assert!(*w > 0 && !buf.is_empty(), "no screen size was reported");

        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        let pixels = buf.as_chunks::<4>().0;
        ppm.extend(pixels.iter().flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write("rdp-frame.ppm", ppm).unwrap();

        let lit = pixels.iter().filter(|p| p[0] | p[1] | p[2] != 0).count();
        println!(
            "{} frames, {lit} non-black pixels -> rdp-frame.ppm",
            tiles.load(std::sync::atomic::Ordering::SeqCst)
        );
        assert!(
            lit > buf.len() / 40,
            "framebuffer came out essentially black - crop is wrong"
        );
    }

    #[test]
    fn a_framebuffer_resized_in_place_is_announced_before_its_pixels() {
        let mut p = Pending {
            size: (4, 4),
            // Long enough ago that the frame isn't held for pacing.
            sent: std::time::Instant::now() - Duration::from_secs(1),
            ..Pending::default()
        };
        let all = |w: u16, h: u16| InclusiveRectangle {
            left: 0,
            top: 0,
            right: w - 1,
            bottom: h - 1,
        };
        p.follow(4, 4);
        p.mark(all(4, 4));
        assert_eq!((p.resized, p.dirty.len()), (None, 1));

        p.follow(6, 2);
        assert_eq!((p.resized, p.dirty.len()), (Some((6, 2)), 0));
        p.mark(all(6, 2));
        let frame = p
            .take(
                &DecodedImage::new(
                    ironrdp_graphics::image_processing::PixelFormat::RgbA32,
                    6,
                    2,
                ),
                true,
            )
            .unwrap();
        let head = |at: usize| u16::from_le_bytes([frame[at], frame[at + 1]]);
        assert_eq!(
            [head(0), head(2), head(4), head(6)],
            [RESIZED, RESIZED, 6, 2]
        );
        assert_eq!([head(8), head(10), head(12), head(14)], [0, 0, 6, 2]);
    }

    #[test]
    fn a_pipeline_is_counted_while_it_lives() {
        let pipelines = Pipelines::default();
        let first = Gfx::new(&pipelines);
        let second = Gfx::new(&pipelines);
        assert_eq!(pipelines.load(std::sync::atomic::Ordering::Relaxed), 2);
        drop(first);
        drop(second);
        assert_eq!(pipelines.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn dirty_regions_merge_only_when_that_sends_no_more_pixels() {
        let r = |left, top, right, bottom| InclusiveRectangle {
            left,
            top,
            right,
            bottom,
        };
        let mut p = Pending::default();
        // A clock in one corner and a cursor in the other stay two.
        p.mark(r(0, 0, 9, 9));
        p.mark(r(1000, 700, 1009, 709));
        assert_eq!(p.dirty.len(), 2);
        // Inside one already there: nothing to add.
        p.mark(r(2, 2, 5, 5));
        assert_eq!(p.dirty.len(), 2);
        // Adjacent halves of a strip are one strip.
        p.mark(r(10, 0, 19, 9));
        assert_eq!(p.dirty.len(), 2);
        assert!(p.dirty.iter().any(|d| (d.left, d.right) == (0, 19)));
        // A region covering everything swallows the lot, including ones already passed.
        p.mark(r(0, 0, 1023, 767));
        assert_eq!(p.dirty.len(), 1);

        // A resize drops what belonged to the old framebuffer.
        p.resize(800, 600);
        assert!(p.dirty.is_empty());
        assert_eq!(p.resized, Some((800, 600)));
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
