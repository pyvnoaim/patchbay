//! The clipboard shared with a remote desktop, over RDP's CLIPRDR channel. Text only:
//! files would mean `CF_HDROP` and chunked file streams. Both directions are lazy, as
//! CLIPRDR is: a copy only announces itself, and the data crosses on paste.

use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FormatDataRequest,
    FormatDataResponse, OwnedFormatDataResponse,
};
use std::sync::mpsc::Sender;

/// Windows hands `CF_UNICODETEXT` over as NUL-terminated UTF-16LE.
fn to_utf16(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain(core::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn from_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&p| u16::from_le_bytes(p))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// The OS's own count of clipboard changes, so the poll compares one integer instead of
/// reading the text every tick. None where there is no counter (X11 and Wayland have
/// none short of XFixes), and the caller falls back to comparing the text.
#[cfg(target_os = "macos")]
pub fn stamp() -> Option<u64> {
    use objc2_app_kit::NSPasteboard;
    Some(NSPasteboard::generalPasteboard().changeCount() as u64)
}

#[cfg(windows)]
pub fn stamp() -> Option<u64> {
    // SAFETY: takes nothing and touches nothing of ours; 0 only when no window station.
    Some(u64::from(unsafe {
        windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
    }))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn stamp() -> Option<u64> {
    None
}

/// A fresh handle per call: holding one across a session blocks other applications
/// on some platforms.
pub fn local_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Also the Bitwarden popup's Copy, which asks the app around the extension to do it.
pub fn set_local_text(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text);
    }
}

/// What the local clipboard last held, as the session knows it. Shared with the poll
/// loop so text that arrived from the remote is not advertised straight back to it.
pub type LastSeen = std::sync::Arc<std::sync::Mutex<Option<String>>>;

/// Our half of CLIPRDR. Never touches the network: callbacks become
/// [`ClipboardMessage`]s on a channel, and the session's pump loop turns those into
/// PDUs, so every socket write stays on the thread that owns the connection.
#[derive(Debug)]
pub struct Backend {
    to_session: Sender<ClipboardMessage>,
    last_seen: LastSeen,
    temp: String,
}

impl Backend {
    pub fn new(to_session: Sender<ClipboardMessage>, last_seen: LastSeen) -> Self {
        Self {
            to_session,
            last_seen,
            temp: std::env::temp_dir().to_string_lossy().into_owned(),
        }
    }

    /// The formats served: announced when the local clipboard changes and on request.
    pub fn text_formats() -> Vec<ClipboardFormat> {
        vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
    }

    fn send(&self, msg: ClipboardMessage) {
        // A closed session is the normal end, not an error.
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
        // No long format names, no file streams: one well-known format.
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        // Offer what is already on the clipboard, so the first paste works.
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

    /// Something was copied remotely. Ask for it as Unicode text if offered; other
    /// formats leave the local clipboard alone.
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        if available_formats
            .iter()
            .any(|f| f.id == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    /// The remote is pasting what was advertised: hand over the text now.
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let response = match (request.format, local_text()) {
            (ClipboardFormatId::CF_UNICODETEXT, Some(text)) => {
                FormatDataResponse::new_data(to_utf16(&text))
            }
            _ => FormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response.into_owned()));
    }

    /// Remote text has arrived: put it on this machine's clipboard.
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
        // Windows sends the terminator inside the payload.
        assert_eq!(from_utf16(&to_utf16("ok")), "ok");
        assert_eq!(
            to_utf16("ok").len(),
            6,
            "two chars plus the NUL, two bytes each"
        );
    }

    #[test]
    fn junk_decodes_to_something_rather_than_panicking() {
        // An odd byte count and a lone surrogate are both invalid; neither may panic.
        assert_eq!(from_utf16(&[0x41]), "");
        let _ = from_utf16(&[0x00, 0xD8, 0x41, 0x00]);
    }
}
