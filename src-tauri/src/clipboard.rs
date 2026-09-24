//! The clipboard shared with a remote desktop, over RDP's CLIPRDR channel: text, and
//! files as CLIPRDR's file lists and streams. Local to remote is lazy, as CLIPRDR is: a
//! copy only announces itself and the data crosses on paste. Remote files to local are
//! not, because nothing tells us when this machine pastes.

use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    ClipboardGeneralCapabilityFlags, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor, FormatDataRequest, FormatDataResponse, OwnedFormatDataResponse,
};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
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

/// What the local clipboard holds, as far as a remote desktop is concerned.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Text(String),
    Files(Vec<PathBuf>),
}

/// Files first: Finder puts their names beside them as text, and a paste of those
/// names is not what a copied file means.
pub fn local() -> Option<Content> {
    let mut c = arboard::Clipboard::new().ok()?;
    match c.get().file_list() {
        Ok(files) if !files.is_empty() => Some(Content::Files(files)),
        _ => c.get_text().ok().map(Content::Text),
    }
}

/// The clipboard as the session knows it, shared between the poll loop and the
/// backend so what arrived from the remote is not offered straight back to it.
#[derive(Debug, Default)]
pub struct State {
    pub seen: Option<Content>,
    /// The local path behind each descriptor last offered: the remote asks by index.
    offered: Vec<PathBuf>,
}

pub type LastSeen = std::sync::Arc<std::sync::Mutex<State>>;

/// Starting from what is on the clipboard now: `on_ready` offers that, and the poll
/// would offer it a second time.
pub fn last_seen() -> LastSeen {
    std::sync::Arc::new(std::sync::Mutex::new(State {
        seen: local(),
        offered: Vec::new(),
    }))
}

/// The message that puts `content` on offer to the remote.
pub fn offer(content: &Content, state: &mut State) -> ClipboardMessage {
    match content {
        Content::Text(_) => ClipboardMessage::SendInitiateCopy(Backend::text_formats()),
        Content::Files(paths) => {
            let (descriptors, offered) = describe(paths);
            state.offered = offered;
            ClipboardMessage::SendInitiateFileCopy(descriptors)
        }
    }
}

/// CLIPRDR's longest name, the relative path included.
const MAX_NAME: usize = 259;
// ponytail: every local copy of files is walked while a session is open, on its
// thread, whether or not it is ever pasted there - so a copied home folder is offered
// as nothing rather than frozen on. A walk off the thread if a real folder hits it.
const MAX_ENTRIES: usize = 10_000;

/// The files as CLIPRDR lists them, folders walked, with the local path behind each in
/// the same order. Anything the library would drop is left out here instead, or every
/// index after it would name the wrong file.
fn describe(paths: &[PathBuf]) -> (Vec<FileDescriptor>, Vec<PathBuf>) {
    let mut out = (Vec::new(), Vec::new());
    for p in paths {
        walk(p, None, &mut out);
    }
    if out.0.len() > MAX_ENTRIES {
        return Default::default();
    }
    out
}

fn walk(path: &Path, parent: Option<&str>, out: &mut (Vec<FileDescriptor>, Vec<PathBuf>)) {
    if out.0.len() > MAX_ENTRIES {
        return;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let wire = match parent {
        Some(p) => format!("{p}\\{name}"),
        None => name.to_owned(),
    };
    // Not followed: a link inside a copied folder can point back up it.
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if wire.chars().count() > MAX_NAME {
        return;
    }
    let mut d = FileDescriptor::new(name);
    if let Some(p) = parent {
        d = d.with_relative_path(p);
    }
    if meta.is_dir() {
        out.0
            .push(d.with_attributes(ClipboardFileAttributes::DIRECTORY));
        out.1.push(path.to_owned());
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            walk(&entry.path(), Some(&wire), out);
        }
    } else if meta.is_file() {
        out.0.push(
            d.with_attributes(ClipboardFileAttributes::NORMAL)
                .with_file_size(meta.len()),
        );
        out.1.push(path.to_owned());
    }
}

/// Answer the remote reading one of the files offered.
fn serve(path: &Path, req: &FileContentsRequest) -> std::io::Result<FileContentsResponse<'static>> {
    let mut f = std::fs::File::open(path)?;
    if req.flags.contains(FileContentsFlags::SIZE) {
        return Ok(FileContentsResponse::new_size_response(
            req.stream_id,
            f.metadata()?.len(),
        ));
    }
    f.seek(SeekFrom::Start(req.position))?;
    let mut buf = Vec::new();
    f.take(u64::from(req.requested_size))
        .read_to_end(&mut buf)?;
    Ok(FileContentsResponse::new_data_response(req.stream_id, buf))
}

// ponytail: the Mac clipboard can't be asked for later, so a remote copy of files is
// downloaded the moment it is made, and past this it is left on the remote. A file
// promise (NSFilePromiseProvider) is the upgrade if the cap gets in the way.
const MAX_DOWNLOAD: u64 = 1 << 30;
const CHUNK: u32 = 1 << 20;

/// A remote copy of files, fetched into a folder of ours one file at a time.
#[derive(Debug)]
struct Download {
    /// What goes on the local clipboard once everything is in: the top-level items.
    roots: Vec<PathBuf>,
    /// Remote index, where it lands, and its size once known.
    files: Vec<(i32, PathBuf, Option<u64>)>,
    at: usize,
    out: Option<std::fs::File>,
    written: u64,
    total: u64,
    stream: u32,
}

/// Lay out a remote file list under `dir`: folders made, files queued. None when
/// anything in it can't land there, or it is more than will be fetched.
fn fetch(files: &[FileDescriptor], dir: Option<PathBuf>) -> Option<Download> {
    let dir = dir?;
    let mut d = Download {
        roots: Vec::new(),
        files: Vec::new(),
        at: 0,
        out: None,
        written: 0,
        total: 0,
        stream: 0,
    };
    for (index, f) in files.iter().enumerate() {
        let dest = landing(&dir, f)?;
        if f.relative_path.as_deref().unwrap_or("").is_empty() {
            d.roots.push(dest.clone());
        }
        if f.attributes
            .is_some_and(|a| a.contains(ClipboardFileAttributes::DIRECTORY))
        {
            std::fs::create_dir_all(&dest).ok()?;
        } else {
            std::fs::create_dir_all(dest.parent()?).ok()?;
            d.total = d.total.saturating_add(f.file_size.unwrap_or(0));
            d.files
                .push((i32::try_from(index).ok()?, dest, f.file_size));
        }
    }
    (d.total <= MAX_DOWNLOAD).then_some(d)
}

impl Download {
    /// The request for the next piece, opening the next file as needed. None once
    /// every file is in; Err for a file that can't be created here.
    fn request(&mut self) -> Result<Option<FileContentsRequest>, ()> {
        while let Some((index, dest, size)) = self.files.get(self.at).cloned() {
            if self.out.is_none() {
                self.out = Some(std::fs::File::create(dest).map_err(|_| ())?);
            }
            let (flags, position, requested_size) = match size {
                None => (FileContentsFlags::SIZE, 0, 8),
                Some(size) if self.written < size => (
                    FileContentsFlags::RANGE,
                    self.written,
                    u32::try_from(size - self.written).map_or(CHUNK, |n| n.min(CHUNK)),
                ),
                Some(_) => {
                    self.out = None;
                    self.written = 0;
                    self.at += 1;
                    continue;
                }
            };
            return Ok(Some(FileContentsRequest {
                stream_id: self.stream,
                index,
                flags,
                position,
                requested_size,
                data_id: None,
            }));
        }
        Ok(None)
    }

    /// Take the answer to the last request: a size, or the bytes of a range. False
    /// ends the download.
    fn take(&mut self, data: &[u8]) -> bool {
        let Some((_, _, size)) = self.files.get_mut(self.at) else {
            return false;
        };
        match size {
            None => {
                let Ok(n) = <[u8; 8]>::try_from(data).map(u64::from_le_bytes) else {
                    return false;
                };
                *size = Some(n);
                self.total = self.total.saturating_add(n);
                self.total <= MAX_DOWNLOAD
            }
            // An empty answer short of the end would ask for the same range forever,
            // and one longer than asked for would run past the size.
            Some(size) => {
                let fits = !data.is_empty() && data.len() as u64 <= *size - self.written;
                if fits && self.out.as_mut().is_some_and(|f| f.write_all(data).is_ok()) {
                    self.written += data.len() as u64;
                    true
                } else {
                    false
                }
            }
        }
    }
}

/// A remote file's place under `dir`, or None for a name with anything in it but
/// plain components. The library already sanitizes these; this is the local check.
fn landing(dir: &Path, f: &FileDescriptor) -> Option<PathBuf> {
    let mut out = dir.to_owned();
    let parts = f.relative_path.iter().flat_map(|p| p.split('\\'));
    for part in parts.chain(std::iter::once(f.name.as_str())) {
        let mut c = Path::new(part).components();
        match (c.next(), c.next()) {
            (Some(std::path::Component::Normal(n)), None) => out.push(n),
            _ => return None,
        }
        if cfg!(windows) && ironrdp_cliprdr::is_windows_device_name(part) {
            return None;
        }
    }
    Some(out)
}

/// Our half of CLIPRDR. Never touches the network: callbacks become
/// [`ClipboardMessage`]s on a channel, and the session's pump loop turns those into
/// PDUs, so every socket write stays on the thread that owns the connection.
#[derive(Debug)]
pub struct Backend {
    to_session: Sender<ClipboardMessage>,
    last_seen: LastSeen,
    temp: String,
    download: Option<Download>,
    stream: u32,
}

impl Backend {
    pub fn new(to_session: Sender<ClipboardMessage>, last_seen: LastSeen) -> Self {
        Self {
            to_session,
            last_seen,
            temp: std::env::temp_dir().to_string_lossy().into_owned(),
            download: None,
            stream: 0,
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

    fn offer_local(&self) {
        if let Some(c) = local() {
            let msg = offer(&c, &mut self.last_seen.lock().unwrap());
            self.send(msg);
        }
    }

    /// One folder per session, emptied by each remote copy: what was pasted from the
    /// last one has been copied out of it by then.
    fn landing_dir(&self) -> std::io::Result<PathBuf> {
        let dir = Path::new(&self.temp).join(format!("patchbay-clipboard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        // Canonical, so the paths read back off the clipboard compare equal: /var is a
        // link to /private/var on macOS.
        std::fs::canonicalize(dir)
    }

    /// Ask for the next piece of the download, or hand it to the clipboard when done.
    fn next(&mut self) {
        let Some(d) = self.download.as_mut() else {
            return;
        };
        self.stream = self.stream.wrapping_add(1);
        d.stream = self.stream;
        match d.request() {
            Ok(Some(request)) => self.send(ClipboardMessage::SendFileContentsRequest(request)),
            Ok(None) => {
                let roots = std::mem::take(&mut d.roots);
                self.download = None;
                if let Ok(mut c) = arboard::Clipboard::new() {
                    if c.set().file_list(&roots).is_ok() {
                        self.last_seen.lock().unwrap().seen = Some(Content::Files(roots));
                    }
                }
            }
            Err(()) => self.download = None,
        }
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
        // Long names because the file list is known by one: "FileGroupDescriptorW" is
        // longer than the short form holds. No locking: a remote copy is fetched at once.
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
    }

    fn on_ready(&mut self) {
        // Offer what is already on the clipboard, so the first paste works.
        self.offer_local();
    }

    fn on_request_format_list(&mut self) {
        self.offer_local();
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    /// Something was copied remotely. Files are fetched now, text on offer is asked
    /// for as Unicode; anything else leaves the local clipboard alone.
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.download = None;
        if let Some(files) = available_formats.iter().find(|f| {
            f.name()
                .is_some_and(|n| n.value() == ClipboardFormatName::FILE_LIST.value())
        }) {
            self.send(ClipboardMessage::SendInitiatePaste(files.id));
        } else if available_formats
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
        let response = match (request.format, local()) {
            (ClipboardFormatId::CF_UNICODETEXT, Some(Content::Text(text))) => {
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
            self.last_seen.lock().unwrap().seen = Some(Content::Text(text));
        }
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], _: Option<u32>) {
        self.download = fetch(files, self.landing_dir().ok());
        self.next();
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        let path = usize::try_from(request.index)
            .ok()
            .and_then(|i| self.last_seen.lock().unwrap().offered.get(i).cloned());
        let response = path
            .and_then(|p| serve(&p, &request).ok())
            .unwrap_or_else(|| FileContentsResponse::new_error(request.stream_id));
        self.send(ClipboardMessage::SendFileContentsResponse(response));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let taken = self.download.as_mut().is_some_and(|d| {
            response.stream_id() == d.stream && !response.is_error() && d.take(response.data())
        });
        if !taken {
            self.download = None;
        }
        self.next();
    }
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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("patchbay-clip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_copied_folder_is_listed_with_each_index_naming_its_own_file() {
        let dir = scratch("describe");
        std::fs::create_dir_all(dir.join("logs/old")).unwrap();
        std::fs::write(dir.join("logs/a.txt"), "aaa").unwrap();
        std::fs::write(dir.join("logs/old/b.txt"), "b").unwrap();
        std::fs::write(dir.join("top.txt"), "t").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&dir, dir.join("logs/loop")).unwrap();

        let (descs, paths) = describe(&[dir.join("logs"), dir.join("top.txt")]);
        assert_eq!(descs.len(), paths.len());
        assert_eq!(descs.len(), 5, "logs, a, old, b, top - and not the link");
        for (d, p) in descs.iter().zip(&paths) {
            assert_eq!(
                Some(d.name.as_str()),
                p.file_name().and_then(|n| n.to_str())
            );
            let is_dir = d
                .attributes
                .is_some_and(|a| a.contains(ClipboardFileAttributes::DIRECTORY));
            assert_eq!(is_dir, p.is_dir(), "{p:?}");
        }
        let b = descs.iter().find(|d| d.name == "b.txt").unwrap();
        assert_eq!(b.relative_path.as_deref(), Some(r"logs\old"));
        assert_eq!(b.file_size, Some(1));
        assert!(descs
            .iter()
            .find(|d| d.name == "top.txt")
            .unwrap()
            .relative_path
            .is_none());
    }

    #[test]
    fn a_name_too_long_for_the_wire_is_left_out_rather_than_shifting_the_rest() {
        let dir = scratch("long");
        // Long by its path, since no file system takes a 260-character name.
        let folder = dir.join("d".repeat(200));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("f".repeat(70)), "").unwrap();
        std::fs::write(folder.join("ok.txt"), "").unwrap();
        let (descs, paths) = describe(std::slice::from_ref(&folder));
        assert_eq!(descs.len(), paths.len());
        assert_eq!(paths, vec![folder.clone(), folder.join("ok.txt")]);
    }

    #[test]
    fn the_remote_reads_a_size_and_then_a_range() {
        let dir = scratch("serve");
        let f = dir.join("f");
        std::fs::write(&f, "hello world").unwrap();
        let ask = |flags, position, requested_size| FileContentsRequest {
            stream_id: 7,
            index: 0,
            flags,
            position,
            requested_size,
            data_id: None,
        };
        let size = serve(&f, &ask(FileContentsFlags::SIZE, 0, 8)).unwrap();
        assert_eq!(size.data_as_size().unwrap(), 11);
        let range = serve(&f, &ask(FileContentsFlags::RANGE, 6, 100)).unwrap();
        assert_eq!(range.data(), b"world");
        assert_eq!(range.stream_id(), 7);
    }

    #[test]
    fn a_remote_name_cannot_land_outside_the_folder() {
        let dir = PathBuf::from("/tmp/x");
        let d = |name: &str, rel: Option<&str>| {
            let f = FileDescriptor::new(name);
            match rel {
                Some(r) => f.with_relative_path(r),
                None => f,
            }
        };
        assert_eq!(
            landing(&dir, &d("a.txt", Some(r"sub\deep"))),
            Some(dir.join("sub/deep/a.txt"))
        );
        assert_eq!(landing(&dir, &d("..", None)), None);
        assert_eq!(landing(&dir, &d("a.txt", Some(r"..\.."))), None);
        assert_eq!(landing(&dir, &d("/etc/passwd", None)), None);
        assert_eq!(landing(&dir, &d("a/b", None)), None);
    }

    #[test]
    fn a_remote_copy_is_fetched_file_by_file_in_chunks() {
        let dir = scratch("fetch");
        let files = [
            FileDescriptor::new("docs").with_attributes(ClipboardFileAttributes::DIRECTORY),
            FileDescriptor::new("a.bin")
                .with_relative_path("docs")
                .with_file_size(3),
            FileDescriptor::new("empty").with_file_size(0),
            FileDescriptor::new("unsized"),
        ];
        let mut d = fetch(&files, Some(dir.clone())).unwrap();
        assert_eq!(
            d.roots,
            vec![dir.join("docs"), dir.join("empty"), dir.join("unsized")]
        );

        let r = d.request().unwrap().unwrap();
        assert_eq!(
            (r.index, r.flags, r.position, r.requested_size),
            (1, FileContentsFlags::RANGE, 0, 3)
        );
        assert!(!d.take(b"abcd"), "more than the file holds");
        assert!(d.take(b"ab"));
        let r = d.request().unwrap().unwrap();
        assert_eq!((r.position, r.requested_size), (2, 1));
        assert!(d.take(b"c"));

        // The empty file needs no request; the unsized one asks its size first.
        let r = d.request().unwrap().unwrap();
        assert_eq!((r.index, r.flags), (3, FileContentsFlags::SIZE));
        assert_eq!(std::fs::read(dir.join("docs/a.bin")).unwrap(), b"abc");
        assert!(dir.join("empty").is_file());
        assert!(d.take(&2u64.to_le_bytes()));
        let r = d.request().unwrap().unwrap();
        assert_eq!(
            (r.index, r.flags, r.requested_size),
            (3, FileContentsFlags::RANGE, 2)
        );
        assert!(!d.take(b""), "an empty range would loop");
    }

    #[test]
    fn more_than_the_cap_is_not_fetched_at_all() {
        let dir = scratch("cap");
        let files = [FileDescriptor::new("big").with_file_size(MAX_DOWNLOAD + 1)];
        assert!(fetch(&files, Some(dir)).is_none());
    }
}
