//! VNC in a tab: vnc-rs (pure Rust) decoding into a framebuffer of ours, which reaches
//! the window as the same tile records `rdp_session.rs` sends, paced by its `Pending`.
//! The handoff to `vnc://` in `commands/remote.rs` stays, for what this can't open.
//!
//! The argument `rdp.rs` asked for: Windows and Linux ship no VNC viewer, and the boxes
//! VNC is pointed at (hypervisor consoles, KVMs, Linux desktops) were the only devices
//! that still left the window. A Mac target is the exception: Screen Sharing signs in
//! with Apple's own scheme, which vnc-rs doesn't speak, so a server offering nothing
//! else fails with `UNSUPPORTED` and the window hands it to the system viewer instead.
//!
//! vnc-rs is async and spawns its reader tasks with `tokio::spawn`, so each session owns
//! a current-thread runtime and the pump runs inside `block_on` - the tasks only move
//! while it does. A refresh is asked for once an update has arrived and the window has
//! painted the last frame, so a slow window gets fewer frames rather than a backlog; an
//! idle desktop is still asked every `STALL` in case a request went unanswered.
//!
//! The desktop is never resized from here. A VNC server is often a screen someone is
//! sitting at, and ExtendedDesktopSize would resize it under them; the canvas letterboxes.
//! The cursor is the server's shape (the cursor pseudo-encoding) on the canvas's own
//! pointer, so it moves without a round trip. Local clipboard text goes to the server
//! only when the canvas takes focus: VNC has no "ask on paste", and pushing every copy
//! would hand a password copied in another app to whichever server has a tab open.

use crate::rdp_session::{self, Pending, FRAME, HANDSHAKE, POLL, STALL};
use ironrdp::pdu::geometry::InclusiveRectangle;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use tauri::Emitter as _;
use vnc::{PixelFormat, Rect, VncClient, VncConnector, VncEncoding, VncError, VncEvent, X11Event};

/// What the window can send into a running session. As with RDP, there is no Close:
/// dropping the sender ends the session.
pub enum Input {
    Move {
        x: u16,
        y: u16,
    },
    /// The browser's button number: 0 left, 1 middle, 2 right.
    Button {
        button: u8,
        down: bool,
    },
    /// Windows' wheel units, as the RDP canvas sends them: 120 is a notch, up positive.
    Wheel {
        delta: i16,
    },
    /// The PC/AT scancode the canvas already computes for RDP, plus `KeyboardEvent.key`,
    /// which carries the character the local layout typed.
    Key {
        scancode: u16,
        key: Option<String>,
        down: bool,
    },
    Painted,
    /// The canvas took focus, which is when the local clipboard goes over.
    Focus,
}

pub type Shared = std::sync::Arc<rdp_session::Sessions<Input>>;

/// Matched by the window, which asks for the password and tries again.
pub const NEEDS_PASSWORD: &str = "asks for a VNC password";
/// Matched by the window, which asks again; a saved password that failed is replaced.
pub const REFUSED: &str = "refused the sign-in";
/// Matched by the window, which hands the device to the system's VNC viewer.
pub const UNSUPPORTED: &str = "signs in a way the built-in viewer doesn't speak";

/// The largest desktop taken, as RDP caps it: past this a server is asking for memory.
/// vnc-rs refuses bigger ones itself; this keeps the allocation honest if it ever stops.
const MAX_SIDE: u16 = 8192;
const MAX_PIXELS: usize = 8_294_400;

/// `Ok` for a desktop inside the cap, else the error that ends the session.
fn fits(name: &str, width: u16, height: u16) -> Result<(), String> {
    let ok = width <= MAX_SIDE
        && height <= MAX_SIDE
        && usize::from(width) * usize::from(height) <= MAX_PIXELS;
    ok.then_some(())
        .ok_or_else(|| format!("\"{name}\" asked for a {width}x{height} desktop"))
}

/// A wheel notch in the units `Input::Wheel` carries.
const NOTCH: i32 = 120;

/// A connected session, not yet pumping. Split from [`pump`] so a wrong password fails
/// while the window is still waiting on the command.
pub struct Session {
    runtime: tokio::runtime::Runtime,
    client: VncClient,
    desktop: Desktop,
    name: String,
}

impl Session {
    pub fn size(&self) -> (u16, u16) {
        (self.desktop.width, self.desktop.height)
    }
}

/// Dial `host:port` and sign in. `password` is only called when the server asks for
/// one, so a desktop without a password never touches the keychain. `name` is the
/// device, for the errors the window shows.
pub fn open(
    host: &str,
    port: u16,
    name: &str,
    password: impl FnOnce() -> Option<String> + Send + Sync + 'static,
) -> Result<Session, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let (client, (width, height)) = runtime.block_on(async {
        let stream = tokio::time::timeout(HANDSHAKE, tokio::net::TcpStream::connect((host, port)))
            .await
            .map_err(|_| format!("{host}:{port}: no answer"))?
            .map_err(|e| format!("{host}:{port}: {e}"))?;
        let _ = stream.set_nodelay(true);
        let client = tokio::time::timeout(HANDSHAKE, async {
            VncConnector::new(stream)
                .set_auth_method(async move { password().ok_or(VncError::NoPassword) })
                // In order of preference. Tight is left out: with it a server may send
                // JPEG, which would need a decoder this side.
                .add_encoding(VncEncoding::Zrle)
                .add_encoding(VncEncoding::CopyRect)
                .add_encoding(VncEncoding::Raw)
                .add_encoding(VncEncoding::CursorPseudo)
                .add_encoding(VncEncoding::DesktopSizePseudo)
                .add_encoding(VncEncoding::ExtendedDesktopSizePseudo)
                // Shared, or the server disconnects whoever else is watching.
                .allow_shared(true)
                .set_pixel_format(PixelFormat::rgba())
                .build()?
                .try_start()
                .await?
                .finish()
        })
        .await
        .map_err(|_| format!("\"{name}\" stopped answering while signing in"))?
        .map_err(|e| refusal(e, name, host, port))?;

        // The server's first word after the handshake is the desktop's size.
        loop {
            match tokio::time::timeout(HANDSHAKE, client.recv_event()).await {
                Ok(Ok(VncEvent::SetResolution(s))) => {
                    break fits(name, s.width, s.height).map(|_| (client, (s.width, s.height)))
                }
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => break Err(format!("{host}:{port}: {e}")),
                Err(_) => break Err(format!("\"{name}\" never sent its desktop")),
            }
        }
    })?;

    Ok(Session {
        runtime,
        client,
        desktop: Desktop::new(width, height),
        name: name.to_owned(),
    })
}

/// A failed sign-in, in words the window can act on.
fn refusal(e: VncError, name: &str, host: &str, port: u16) -> String {
    match e {
        VncError::NoPassword => format!("\"{name}\" {NEEDS_PASSWORD}"),
        VncError::WrongPassword => format!("\"{name}\" {REFUSED}: wrong password"),
        // vnc-rs's words for a server that offers only schemes it doesn't speak (Apple's,
        // VeNCrypt, RealVNC's), and what it returns when it recognised none of them.
        VncError::General(m) if m.starts_with("Security type apart from") => {
            format!("\"{name}\" {UNSUPPORTED}")
        }
        VncError::ConnectError => format!("\"{name}\" {UNSUPPORTED}"),
        // The reason an RFB 3.8 server gives for a failed sign-in ("Authentication
        // failure", "Too many authentication failures"). Anything else is a plain error:
        // asking for the password again would replace a saved one for no reason.
        VncError::General(m) if m.to_ascii_lowercase().contains("auth") => {
            format!("\"{name}\" {REFUSED}: {m}")
        }
        VncError::General(m) => format!("\"{name}\": {m}"),
        e => format!("{host}:{port}: {e}"),
    }
}

/// Pump frames to `on_frame` until the server hangs up or the input sender is dropped.
/// Owns its thread, like `rdp_session::pump`, whose record layout the frames share.
pub fn pump(
    session: Session,
    input: Receiver<Input>,
    on_frame: impl Fn(Vec<u8>),
) -> Result<(), String> {
    let Session {
        runtime,
        client,
        mut desktop,
        name,
    } = session;
    let ended = runtime.block_on(run(&client, &mut desktop, &name, input, on_frame));
    let _ = runtime.block_on(client.close());
    ended
}

async fn run(
    client: &VncClient,
    desktop: &mut Desktop,
    name: &str,
    input: Receiver<Input>,
    on_frame: impl Fn(Vec<u8>),
) -> Result<(), String> {
    let lost = |e: VncError| format!("\"{name}\": {e}");
    let mut pointer = Pointer::default();
    // The keysym each held key went down as, so its release matches even when Shift
    // was let go first and the same key would now read as another character.
    let mut held: HashMap<u16, u32> = HashMap::new();
    // What both clipboards last agreed on, so neither side's copy echoes back. Nothing
    // to start with: the first focus sends what is on it, which is the point of focus.
    let mut clip: Option<String> = None;
    let mut asked = std::time::Instant::now();
    let mut fresh = false;
    let mut full = false;

    loop {
        let mut out = Vec::new();
        loop {
            match input.try_recv() {
                Ok(Input::Painted) => desktop.pending.painted = true,
                Ok(Input::Focus) => {
                    let now = crate::clipboard::local_text();
                    if let Some(text) = now.as_deref().filter(|t| Some(*t) != clip.as_deref()) {
                        out.push(X11Event::CopyText(latin(text)));
                    }
                    clip = now;
                }
                Ok(Input::Move { x, y }) => out.push(pointer.moved(x, y)),
                Ok(Input::Button { button, down }) => out.extend(pointer.button(button, down)),
                Ok(Input::Wheel { delta }) => out.extend(pointer.wheel(delta)),
                Ok(Input::Key {
                    scancode,
                    key,
                    down,
                }) => {
                    let sym = if down {
                        let sym = keysym(scancode, key.as_deref());
                        if let Some(sym) = sym {
                            held.insert(scancode, sym);
                        }
                        sym
                    } else {
                        // Only what went down comes up: a dead key never did.
                        held.remove(&scancode)
                    };
                    if let Some(sym) = sym {
                        out.push(X11Event::KeyEvent((sym, down).into()));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        for event in out {
            client.input(event).await.map_err(lost)?;
        }

        if full {
            client.input(X11Event::FullRefresh).await.map_err(lost)?;
            (asked, fresh, full) = (std::time::Instant::now(), false, false);
        } else if (fresh && desktop.pending.painted && asked.elapsed() >= FRAME)
            || asked.elapsed() >= STALL
        {
            client.input(X11Event::Refresh).await.map_err(lost)?;
            (asked, fresh) = (std::time::Instant::now(), false);
        }

        // The timeout is how the input queue gets checked on an idle desktop.
        let quiet = match tokio::time::timeout(POLL, client.recv_event()).await {
            Err(_) => true,
            Ok(first) => {
                let mut next = Some(first.map_err(lost)?);
                while let Some(event) = next {
                    match event {
                        VncEvent::RawImage(rect, data) => {
                            desktop.draw(rect, &data);
                            fresh = true;
                        }
                        VncEvent::Copy(dst, src) => {
                            desktop.copy(dst, src);
                            fresh = true;
                        }
                        VncEvent::SetResolution(s) => {
                            fits(name, s.width, s.height)?;
                            full |= desktop.resize(s.width, s.height);
                        }
                        VncEvent::DesktopUpdate(u) => {
                            if let Some(l) = u.layout {
                                fits(name, l.width, l.height)?;
                                full |= desktop.resize(l.width, l.height);
                            }
                        }
                        VncEvent::SetCursor(rect, data) => desktop.cursor(rect, &data),
                        VncEvent::Text(text) => {
                            crate::clipboard::set_local_text(&text);
                            clip = Some(text);
                        }
                        VncEvent::Error(e) => return Err(format!("\"{name}\": {e}")),
                        _ => {}
                    }
                    // Everything already decoded goes into this frame.
                    next = client.poll_event().await.map_err(lost)?;
                }
                false
            }
        };

        if let Some(frame) =
            desktop
                .pending
                .take(&desktop.pixels, desktop.width, desktop.height, quiet)
        {
            on_frame(frame);
        }
    }
}

/// The framebuffer, RGBA, and what changed in it since the window last heard.
struct Desktop {
    pixels: Vec<u8>,
    width: u16,
    height: u16,
    pending: Pending,
}

impl Desktop {
    fn new(width: u16, height: u16) -> Self {
        let mut d = Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            pending: Pending::default(),
        };
        d.resize(width, height);
        d
    }

    /// `true` when the size changed. Opaque black to start: a VNC pixel format has no
    /// alpha, and a transparent canvas would show the pane behind it.
    fn resize(&mut self, width: u16, height: u16) -> bool {
        if (width, height) == (self.width, self.height) {
            return false;
        }
        (self.width, self.height) = (width, height);
        self.pixels = [0, 0, 0, 255].repeat(usize::from(width) * usize::from(height));
        self.pending.resize(width, height);
        true
    }

    /// The part of `r` inside the framebuffer, or `None` if nothing is. A rectangle comes
    /// off the wire, so it is clamped rather than trusted to fit.
    fn clip(&self, r: Rect) -> Option<(usize, usize, usize, usize)> {
        let (x, y) = (usize::from(r.x), usize::from(r.y));
        let w = usize::from(r.width).min(usize::from(self.width).saturating_sub(x));
        let h = usize::from(r.height).min(usize::from(self.height).saturating_sub(y));
        (w > 0 && h > 0).then_some((x, y, w, h))
    }

    fn mark(&mut self, (x, y, w, h): (usize, usize, usize, usize)) {
        self.pending.mark(InclusiveRectangle {
            left: x as u16,
            top: y as u16,
            right: (x + w - 1) as u16,
            bottom: (y + h - 1) as u16,
        });
    }

    fn draw(&mut self, r: Rect, data: &[u8]) {
        let Some((x, y, w, h)) = self.clip(r) else {
            return;
        };
        let (src_stride, dst_stride) = (usize::from(r.width) * 4, usize::from(self.width) * 4);
        for row in 0..h {
            let Some(src) = data.get(row * src_stride..row * src_stride + w * 4) else {
                break;
            };
            let at = (y + row) * dst_stride + x * 4;
            let dst = &mut self.pixels[at..at + w * 4];
            dst.copy_from_slice(src);
            // The fourth byte is padding on the wire; the canvas reads it as alpha.
            for px in dst.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
        self.mark((x, y, w, h));
    }

    /// CopyRect: a window dragged, moved within the framebuffer rather than resent.
    fn copy(&mut self, dst: Rect, src: Rect) {
        let (Some((dx, dy, w, h)), Some((sx, sy, sw, sh))) = (self.clip(dst), self.clip(src))
        else {
            return;
        };
        let (w, h) = (w.min(sw), h.min(sh));
        let stride = usize::from(self.width) * 4;
        // Rows in the order that never reads one already overwritten.
        let rows: Box<dyn Iterator<Item = usize>> = if dy > sy {
            Box::new((0..h).rev())
        } else {
            Box::new(0..h)
        };
        for row in rows {
            let from = (sy + row) * stride + sx * 4;
            self.pixels
                .copy_within(from..from + w * 4, (dy + row) * stride + dx * 4);
        }
        self.mark((dx, dy, w, h));
    }

    /// The server's pointer shape, with its hotspot as `r.x`, `r.y`. Empty means hidden.
    fn cursor(&mut self, r: Rect, data: &[u8]) {
        self.pending
            .record(&rdp_session::cursor(r.width, r.height, r.x, r.y, data));
    }
}

/// Where the pointer is and which buttons are down: VNC sends all of it on every event.
#[derive(Default)]
struct Pointer {
    x: u16,
    y: u16,
    buttons: u8,
    /// Wheel units short of a whole notch, kept for the next event.
    wheel: i32,
}

impl Pointer {
    fn event(&self, buttons: u8) -> X11Event {
        X11Event::PointerEvent((self.x, self.y, buttons).into())
    }

    fn moved(&mut self, x: u16, y: u16) -> X11Event {
        (self.x, self.y) = (x, y);
        self.event(self.buttons)
    }

    fn button(&mut self, button: u8, down: bool) -> Option<X11Event> {
        let bit = match button {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => return None,
        };
        if down {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        Some(self.event(self.buttons))
    }

    /// A notch is a press and release of button 4 (up) or 5 (down).
    fn wheel(&mut self, delta: i16) -> Vec<X11Event> {
        self.wheel += i32::from(delta);
        let mut out = Vec::new();
        while self.wheel.abs() >= NOTCH {
            let bit = if self.wheel > 0 { 8 } else { 16 };
            out.push(self.event(self.buttons | bit));
            out.push(self.event(self.buttons));
            self.wheel -= NOTCH * self.wheel.signum();
        }
        out
    }
}

/// RFB's cut text is Latin-1 and vnc-rs sends the bytes of a Rust string, so anything
/// past ASCII would land as two characters of mojibake. Line ends are X's.
/// ponytail: ASCII only; the Extended Clipboard pseudo-encoding (UTF-8) if a server
/// that speaks it ever needs accents to cross.
fn latin(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .map(|c| if c.is_ascii() { c } else { '?' })
        .collect()
}

/// The X keysym for a key. A printable `key` is the character the local layout typed,
/// which is what a VNC server wants; otherwise the scancode names the key as it sits on
/// a US keyboard. Either way a server with another layout sees the character, not the
/// position, which is what makes VNC unlike RDP here. A dead key sends nothing: the
/// browser reports the accented character on the key after it.
/// ponytail: AltGr on Windows arrives as Ctrl+Alt, so a server may read its characters
/// as chords; noVNC's AltGr heuristic if someone types `@` that way and it misfires.
fn keysym(scancode: u16, key: Option<&str>) -> Option<u32> {
    if key == Some("Dead") {
        return None;
    }
    let mut chars = key.unwrap_or_default().chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if !c.is_control() {
            return Some(char_sym(c));
        }
    }
    Some(match scancode {
        0x01 => 0xff1b,                                     // Escape
        0x0e => 0xff08,                                     // BackSpace
        0x0f => 0xff09,                                     // Tab
        0x1c => 0xff0d,                                     // Return
        0x1d => 0xffe3,                                     // Control_L
        0x2a => 0xffe1,                                     // Shift_L
        0x36 => 0xffe2,                                     // Shift_R
        0x38 => 0xffe9,                                     // Alt_L
        0x3a => 0xffe5,                                     // Caps_Lock
        0x3b..=0x44 => 0xffbe + u32::from(scancode - 0x3b), // F1-F10
        0x57 => 0xffc8,                                     // F11
        0x58 => 0xffc9,                                     // F12
        0x45 => 0xff7f,                                     // Num_Lock
        0x46 => 0xff14,                                     // Scroll_Lock
        0x37 => 0xffaa,                                     // KP_Multiply
        0x4a => 0xffad,                                     // KP_Subtract
        0x4e => 0xffab,                                     // KP_Add
        0x53 => 0xffae,                                     // KP_Decimal
        0x47..=0x49 => 0xffb7 + u32::from(scancode - 0x47), // KP_7-KP_9
        0x4b..=0x4d => 0xffb4 + u32::from(scancode - 0x4b), // KP_4-KP_6
        0x4f..=0x51 => 0xffb1 + u32::from(scancode - 0x4f), // KP_1-KP_3
        0x52 => 0xffb0,                                     // KP_0
        0xe01d => 0xffe4,                                   // Control_R
        0xe038 => 0xffea,                                   // Alt_R
        0xe035 => 0xffaf,                                   // KP_Divide
        0xe01c => 0xff8d,                                   // KP_Enter
        0xe047 => 0xff50,                                   // Home
        0xe04b => 0xff51,                                   // Left
        0xe048 => 0xff52,                                   // Up
        0xe04d => 0xff53,                                   // Right
        0xe050 => 0xff54,                                   // Down
        0xe049 => 0xff55,                                   // Prior
        0xe051 => 0xff56,                                   // Next
        0xe04f => 0xff57,                                   // End
        0xe052 => 0xff63,                                   // Insert
        0xe053 => 0xffff,                                   // Delete
        0xe05b => 0xffeb,                                   // Super_L
        0xe05c => 0xffec,                                   // Super_R
        _ => char_sym(us_char(scancode)?),
    })
}

/// Latin-1 keysyms are the character itself; everything else is Unicode plus 0x1000000.
fn char_sym(c: char) -> u32 {
    match u32::from(c) {
        n @ (0x20..=0x7e | 0xa0..=0xff) => n,
        n => 0x0100_0000 + n,
    }
}

/// What a printable key types on a US keyboard with nothing held: the fallback when the
/// window sent no character, as for ⌘C turned into Ctrl+C.
fn us_char(scancode: u16) -> Option<char> {
    const ROWS: [(u16, &str); 4] = [
        (0x02, "1234567890-="),
        (0x10, "qwertyuiop[]"),
        (0x1e, "asdfghjkl;'`"),
        (0x2b, "\\zxcvbnm,./"),
    ];
    if scancode == 0x39 {
        return Some(' ');
    }
    ROWS.iter().find_map(|(first, keys)| {
        let i = usize::from(scancode.checked_sub(*first)?);
        keys.chars().nth(i)
    })
}

impl rdp_session::Sessions<Input> {
    /// Connect synchronously, then pump frames into `on_tile` from a thread until closed;
    /// `vnc-exit:<id>` carries why it ended.
    pub fn open(
        &self,
        id: u32,
        session: Session,
        on_tile: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
        app: tauri::AppHandle,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.add(id, tx);
        std::thread::spawn(move || {
            let ended = pump(session, rx, |frame| {
                let _ = on_tile.send(tauri::ipc::InvokeResponseBody::Raw(frame));
            });
            let _ = app.emit(&format!("vnc-exit:{id}"), ended.err());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn px(d: &Desktop, x: usize, y: usize) -> [u8; 4] {
        let at = (y * usize::from(d.width) + x) * 4;
        d.pixels[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn a_key_is_the_character_typed_and_its_release_needs_none() {
        // The layout's character wins over the key's US position: a German Y key.
        assert_eq!(keysym(0x2c, Some("y")), Some(u32::from('y')));
        assert_eq!(keysym(0x1e, Some("A")), Some(u32::from('A')));
        assert_eq!(keysym(0x1a, Some("ü")), Some(0xfc));
        assert_eq!(keysym(0x12, Some("€")), Some(0x0100_20ac));
        // Named keys carry no character.
        assert_eq!(keysym(0x1c, Some("Enter")), Some(0xff0d));
        assert_eq!(keysym(0xe048, Some("ArrowUp")), Some(0xff52));
        assert_eq!(keysym(0x3c, Some("F2")), Some(0xffbf));
        assert_eq!(keysym(0x58, None), Some(0xffc9));
        assert_eq!(keysym(0x4b, None), Some(0xffb4));
        // No character at all (⌘C sent as Ctrl+C, a release): the US key.
        assert_eq!(keysym(0x2e, None), Some(u32::from('c')));
        assert_eq!(keysym(0x0b, None), Some(u32::from('0')));
        assert_eq!(keysym(0x29, None), Some(u32::from('`')));
        assert_eq!(keysym(0x2b, None), Some(u32::from('\\')));
        assert_eq!(keysym(0x35, None), Some(u32::from('/')));
        assert_eq!(keysym(0x39, Some(" ")), Some(0x20));
        // A dead key types nothing until the next key brings its character.
        assert_eq!(keysym(0x0d, Some("Dead")), None);
        assert_eq!(keysym(0x7f, None), None);
    }

    #[test]
    fn tiles_land_clamped_and_opaque() {
        let mut d = Desktop::new(4, 3);
        assert_eq!(px(&d, 0, 0), [0, 0, 0, 255]);
        // A 2x2 tile hanging off the right edge: only its left column lands.
        d.draw(
            rect(3, 1, 2, 2),
            &[1, 2, 3, 0, 9, 9, 9, 0, 4, 5, 6, 0, 9, 9, 9, 0],
        );
        assert_eq!(px(&d, 3, 1), [1, 2, 3, 255]);
        assert_eq!(px(&d, 3, 2), [4, 5, 6, 255]);
        // Wholly outside, or short of data: no panic.
        d.draw(rect(9, 9, 2, 2), &[0; 16]);
        // One row of data for a two-row tile: the row that's there lands.
        d.draw(rect(0, 0, 2, 2), &[7; 8]);
        assert_eq!(px(&d, 1, 0), [7, 7, 7, 255]);
        assert_eq!(px(&d, 1, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn a_desktop_past_the_cap_is_refused() {
        assert!(fits("box", 1920, 1080).is_ok());
        assert!(fits("box", 3840, 2160).is_ok());
        assert!(fits("box", 8193, 10).is_err());
        let err = fits("box", 8192, 8192).unwrap_err();
        assert!(err.contains("8192x8192"), "{err}");
    }

    #[test]
    fn a_copy_onto_itself_moves_every_row_intact() {
        let mut d = Desktop::new(1, 4);
        for (y, v) in [10, 20, 30, 40].into_iter().enumerate() {
            d.draw(rect(0, y as u16, 1, 1), &[v, v, v, 0]);
        }
        // Down by one, overlapping: read bottom-up or row 1 would be copied twice.
        d.copy(rect(0, 1, 1, 3), rect(0, 0, 1, 3));
        let col: Vec<u8> = (0..4).map(|y| px(&d, 0, y)[0]).collect();
        assert_eq!(col, [10, 10, 20, 30]);
        d.copy(rect(0, 0, 1, 3), rect(0, 1, 1, 3));
        let col: Vec<u8> = (0..4).map(|y| px(&d, 0, y)[0]).collect();
        assert_eq!(col, [10, 20, 30, 30]);
    }

    #[test]
    fn the_wheel_sends_whole_notches_and_keeps_the_rest() {
        let mut p = Pointer::default();
        p.button(0, true);
        assert!(p.wheel(60).is_empty());
        let notch = p.wheel(60);
        assert_eq!(notch.len(), 2);
        let buttons = |e: &X11Event| match e {
            X11Event::PointerEvent(m) => m.bottons,
            _ => panic!("not a pointer event"),
        };
        // Up is button 4, and the held left button stays held through it.
        assert_eq!(buttons(&notch[0]), 1 | 8);
        assert_eq!(buttons(&notch[1]), 1);
        assert_eq!(p.wheel(-240).len(), 4);
        assert_eq!(p.button(3, true).map(|e| buttons(&e)), None);
    }

    #[test]
    fn clipboard_text_crosses_as_ascii() {
        assert_eq!(latin("a\r\nb"), "a\nb");
        assert_eq!(latin("grüße"), "gr??e");
    }

    /// Just enough of an RFB 3.8 server to stand in for one: offers `security`, and on
    /// "None" serves a 4x2 desktop, answers the first update request with one 2x1 Raw
    /// tile at (1, 0), and passes every key it is sent back as (keysym, down).
    fn fake_server(security: &'static [u8]) -> (u16, std::sync::mpsc::Receiver<(u32, bool)>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (keys, got) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let take = |s: &mut std::net::TcpStream, n: usize| {
                let mut b = vec![0; n];
                s.read_exact(&mut b).map(|_| b)
            };
            s.write_all(b"RFB 003.008\n").unwrap();
            take(&mut s, 12).unwrap();
            s.write_all(&[security.len() as u8]).unwrap();
            s.write_all(security).unwrap();
            if security != [1] {
                // Whatever the client does next, the test has seen what it needed.
                let _ = take(&mut s, 1);
                return;
            }
            take(&mut s, 1).unwrap();
            s.write_all(&0u32.to_be_bytes()).unwrap();
            take(&mut s, 1).unwrap(); // ClientInit
            let mut init = vec![0, 4, 0, 2]; // 4x2
            init.extend([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
            init.extend(4u32.to_be_bytes());
            init.extend(b"fake");
            s.write_all(&init).unwrap();
            let mut served = false;
            while let Ok(t) = take(&mut s, 1) {
                let rest = match t[0] {
                    0 => 19,
                    2 => {
                        let b = take(&mut s, 3).unwrap();
                        4 * usize::from(u16::from_be_bytes([b[1], b[2]]))
                    }
                    3 => 9,
                    4 => {
                        let b = take(&mut s, 7).unwrap();
                        let sym = u32::from_be_bytes([b[3], b[4], b[5], b[6]]);
                        let _ = keys.send((sym, b[0] == 1));
                        0
                    }
                    5 => 5,
                    6 => {
                        let b = take(&mut s, 7).unwrap();
                        usize::from(u16::from_be_bytes([b[5], b[6]]))
                    }
                    _ => return,
                };
                take(&mut s, rest).unwrap();
                if t[0] == 3 && !served {
                    served = true;
                    // One rect: (1, 0) 2x1, Raw, in the client's RGBA with no alpha.
                    let mut up = vec![0, 0, 0, 1, 0, 1, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0];
                    up.extend([10, 20, 30, 0, 40, 50, 60, 0]);
                    s.write_all(&up).unwrap();
                }
            }
        });
        (port, got)
    }

    #[test]
    fn a_desktop_reaches_the_window_and_a_key_reaches_the_desktop() {
        let (port, keys) = fake_server(&[1]);
        let session = open("127.0.0.1", port, "fake", || None).expect("connect");
        assert_eq!(session.size(), (4, 2));

        let (tx, rx) = std::sync::mpsc::channel();
        let (frames, frame) = std::sync::mpsc::channel();
        let pumping = std::thread::spawn(move || {
            pump(session, rx, |f| {
                let _ = frames.send(f);
            })
        });

        let f = frame
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a frame");
        let at = |i: usize| u16::from_le_bytes([f[i], f[i + 1]]);
        // The size first, then the tile, opaque.
        assert_eq!([at(0), at(2), at(4), at(6)], [u16::MAX, u16::MAX, 4, 2]);
        assert_eq!([at(8), at(10), at(12), at(14)], [1, 0, 2, 1]);
        assert_eq!(f[16..24], [10, 20, 30, 255, 40, 50, 60, 255]);

        // ⇧A down, A up after ⇧ is let go: the release is still the A that went down.
        for (scancode, key, down) in [(0x2a, "Shift", true), (0x1e, "A", true)] {
            tx.send(Input::Key {
                scancode,
                key: Some(key.into()),
                down,
            })
            .unwrap();
        }
        for (scancode, key) in [(0x2a, "Shift"), (0x1e, "a")] {
            tx.send(Input::Key {
                scancode,
                key: Some(key.into()),
                down: false,
            })
            .unwrap();
        }
        let got: Vec<_> = (0..4)
            .map(|_| {
                keys.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
            })
            .collect();
        assert_eq!(
            got,
            [(0xffe1, true), (0x41, true), (0xffe1, false), (0x41, false)]
        );

        drop(tx);
        assert!(pumping.join().unwrap().is_ok());
    }

    #[test]
    fn a_server_that_wants_a_password_or_apples_sign_in_says_so() {
        let (port, _) = fake_server(&[2]);
        let err = open("127.0.0.1", port, "kvm", || None).err().unwrap();
        assert!(err.contains(NEEDS_PASSWORD), "got {err}");
        // 30 is Apple's, which vnc-rs doesn't know at all; 19 is VeNCrypt, which it
        // knows of and doesn't speak.
        for only in [&[30u8][..], &[19]] {
            let (port, _) = fake_server(only);
            let err = open("127.0.0.1", port, "mac", || None).err().unwrap();
            assert!(err.contains(UNSUPPORTED), "{only:?}: got {err}");
        }
    }

    #[test]
    fn a_refused_connection_names_the_host_and_port() {
        // Nothing listens on port 1, so this fails at TCP connect.
        let err = open("127.0.0.1", 1, "box", || None)
            .err()
            .expect("nothing listens on port 1");
        assert!(err.contains("127.0.0.1:1"), "got {err}");
    }

    #[test]
    fn a_sign_in_failure_says_what_the_window_should_do() {
        let r = |e| refusal(e, "kvm", "10.0.0.7", 5900);
        assert!(r(VncError::NoPassword).contains(NEEDS_PASSWORD));
        assert!(r(VncError::WrongPassword).contains(REFUSED));
        assert!(r(VncError::General("Authentication failed".into())).contains(REFUSED));
        assert!(!r(VncError::General("Channel closed".into())).contains(REFUSED));
        assert!(r(VncError::ConnectError).contains(UNSUPPORTED));
        let apple = "Security type apart from Vnc Auth has not been implemented";
        assert!(r(VncError::General(apple.into())).contains(UNSUPPORTED));
    }
}
