//! The clipboard shared with a remote desktop, over RDP's CLIPRDR channel.
//!
//! Text only, deliberately. Files mean `CF_HDROP`, a temp directory and chunked
//! file-stream transfers - a much larger feature, and copying an error message out
//! of a Windows box is what people actually open a session for.
//!
//! Both directions are lazy, which is how CLIPRDR works: whoever copies only
//! *announces* that it has something, and the data crosses when the other side
//! pastes. So a copy on a remote desktop costs nothing until you use it.

use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FormatDataRequest,
    FormatDataResponse, OwnedFormatDataResponse,
};
use std::sync::mpsc::Sender;

/// Windows hands `CF_UNICODETEXT` over as NUL-terminated UTF-16LE.
fn to_utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(core::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

fn from_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Reading and writing the machine's own clipboard. Each call opens its own handle:
/// holding one across a whole session blocks other applications on some platforms,
/// and this happens at human speed, not in a loop.
pub fn local_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn set_local_text(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text);
    }
}

/// Our half of CLIPRDR. It never talks to the network - it turns callbacks into
/// [`ClipboardMessage`]s on a channel, and the session's pump loop turns those into
/// PDUs. That keeps every socket write on the one thread that owns the connection.
/// What the local clipboard last held, as far as the session is concerned. Shared
/// with the poll loop so text that arrived *from* the remote isn't immediately
/// advertised back to it - that round trip is wasted, and it takes clipboard
/// ownership away from the machine that actually has the data.
pub type LastSeen = std::sync::Arc<std::sync::Mutex<Option<String>>>;

#[derive(Debug)]
pub struct Backend {
    to_session: Sender<ClipboardMessage>,
    last_seen: LastSeen,
    temp: String,
}

impl Backend {
    pub fn new(to_session: Sender<ClipboardMessage>, last_seen: LastSeen) -> Self {
        Self { to_session, last_seen, temp: std::env::temp_dir().to_string_lossy().into_owned() }
    }

    /// The formats we can serve. Announced when our clipboard changes, and again
    /// whenever the remote asks what we have.
    pub fn text_formats() -> Vec<ClipboardFormat> {
        vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
    }

    fn send(&self, msg: ClipboardMessage) {
        // A closed session is the normal way this ends, not an error to report.
        let _ = self.to_session.send(msg);
    }
}

impl ironrdp_core::AsAny for Backend {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        &self.temp
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // No long format names, no file streams: we serve one well-known format.
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        // Offer whatever is already on the clipboard, so the first paste into the
        // session works without having to copy something again first.
        if local_text().is_some() {
            self.send(ClipboardMessage::SendInitiateCopy(Self::text_formats()));
        }
    }

    fn on_request_format_list(&mut self) {
        if local_text().is_some() {
            self.send(ClipboardMessage::SendInitiateCopy(Self::text_formats()));
        }
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    /// Something was copied over there. Ask for it as Unicode text if that's on
    /// offer; anything else we can't represent, so we leave the local clipboard be.
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if available_formats.iter().any(|f| f.id == ClipboardFormatId::CF_UNICODETEXT) {
            self.send(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
        }
    }

    /// They're pasting what we advertised, so hand over the text now.
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = match (request.format, local_text()) {
            (ClipboardFormatId::CF_UNICODETEXT, Some(text)) => {
                FormatDataResponse::new_data(to_utf16(&text))
            }
            _ => FormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response.into_owned()));
    }

    /// Their text has arrived - put it on this machine's clipboard.
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        let text = from_utf16(response.data());
        if !text.is_empty() {
            set_local_text(&text);
            *self.last_seen.lock().unwrap() = Some(text);
        }
    }

    fn on_file_contents_request(&mut self, _: ironrdp_cliprdr::pdu::FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: ironrdp_cliprdr::pdu::FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: ironrdp_cliprdr::pdu::LockDataId) {}
    fn on_unlock(&mut self, _: ironrdp_cliprdr::pdu::LockDataId) {}
}

/// Marker so the caller can hold the owned response without fighting lifetimes.
trait IntoOwnedResponse {
    fn into_owned(self) -> OwnedFormatDataResponse;
}

impl IntoOwnedResponse for FormatDataResponse<'_> {
    fn into_owned(self) -> OwnedFormatDataResponse {
        if self.is_error() {
            FormatDataResponse::new_error()
        } else {
            FormatDataResponse::new_data(self.data().to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_survives_the_round_trip_through_windows_encoding() {
        for s in ["hello", "", "grüße", "日本語", "line\r\nbreak"] {
            assert_eq!(from_utf16(&to_utf16(s)), s, "round trip of {s:?}");
        }
    }

    #[test]
    fn a_trailing_nul_is_not_part_of_the_text() {
        // Windows sends the terminator inside the payload; keeping it would append a
        // stray character to everything pasted out of a session.
        assert_eq!(from_utf16(&to_utf16("ok")), "ok");
        assert_eq!(to_utf16("ok").len(), 6, "two chars plus the NUL, two bytes each");
    }

    #[test]
    fn junk_decodes_to_something_rather_than_panicking() {
        // An odd byte count can't be UTF-16 at all; a lone surrogate isn't valid
        // either. Neither should take the session down.
        assert_eq!(from_utf16(&[0x41]), "");
        let _ = from_utf16(&[0x00, 0xD8, 0x41, 0x00]);
    }
}
