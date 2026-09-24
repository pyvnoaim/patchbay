//! A folder of this machine shared into the remote desktop as a drive, over RDP's
//! RDPDR channel: `\\tsclient\transfer` on the far end, `~/Downloads/patchbay` here.
//! One folder, never the home directory - the far end reads and writes whatever is in it.

use std::path::{Path, PathBuf};

/// What Explorer calls it: "transfer on patchbay".
pub const DRIVE: &str = "transfer";

pub fn folder() -> PathBuf {
    crate::sftp::downloads().join("patchbay")
}

/// The server's path with every `.` and `..` dropped, always rooted. The stock backend
/// appends it to the folder as written, so `\..\..\.ssh\authorized_keys` would land
/// outside it - and a server is somebody else's machine.
#[cfg_attr(not(unix), allow(dead_code))] // only the unix backend has paths to confine
pub fn confine(path: &str) -> String {
    path.split(['\\', '/'])
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .fold(String::new(), |out, c| out + "\\" + c)
}

/// Files dropped on a remote desktop tab, copied into the shared folder. Files only: a
/// folder would be a recursive copy for what the far end can already reach by path.
pub fn copy_in(paths: &[String]) -> Result<usize, String> {
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for p in paths {
        let from = Path::new(p);
        let name = from
            .file_name()
            .filter(|_| from.is_file())
            .ok_or_else(|| format!("\"{p}\" isn't a file; only files can be dropped"))?;
        std::fs::copy(from, dir.join(name)).map_err(|e| format!("\"{p}\": {e}"))?;
    }
    Ok(paths.len())
}

#[cfg(unix)]
pub use shared::Folder;

#[cfg(unix)]
mod shared {
    use ironrdp::svc::SvcMessage;
    use ironrdp_pdu::PduResult;
    use ironrdp_rdpdr::pdu::efs::{
        DeviceControlRequest, FileInformationClass, ServerDeviceAnnounceResponse,
        ServerDriveIoRequest,
    };
    use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
    use ironrdp_rdpdr::RdpdrBackend;
    use ironrdp_rdpdr_native::backend::NixRdpdrBackend;

    /// The native backend, with every path the server names put through `confine`
    /// before it sees it.
    #[derive(Debug)]
    pub struct Folder(NixRdpdrBackend);

    impl Folder {
        pub fn new() -> Self {
            let dir = super::folder();
            // Quiet on failure: the drive then lists empty, and a drop says why.
            let _ = std::fs::create_dir_all(&dir);
            Self(NixRdpdrBackend::new(dir.to_string_lossy().into_owned()))
        }
    }

    impl ironrdp_core::AsAny for Folder {
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
            self
        }
    }

    impl RdpdrBackend for Folder {
        fn handle_server_device_announce_response(
            &mut self,
            pdu: ServerDeviceAnnounceResponse,
        ) -> PduResult<()> {
            self.0.handle_server_device_announce_response(pdu)
        }

        fn handle_scard_call(
            &mut self,
            req: DeviceControlRequest<ScardIoCtlCode>,
            call: ScardCall,
        ) -> PduResult<()> {
            self.0.handle_scard_call(req, call)
        }

        fn handle_drive_io_request(
            &mut self,
            mut req: ServerDriveIoRequest,
        ) -> PduResult<Vec<SvcMessage>> {
            match &mut req {
                ServerDriveIoRequest::ServerCreateDriveRequest(r) => {
                    r.path = super::confine(&r.path)
                }
                ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => {
                    r.path = super::confine(&r.path)
                }
                ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => {
                    if let FileInformationClass::Rename(to) = &mut r.set_buffer {
                        to.file_name = super::confine(&to.file_name);
                    }
                }
                _ => {}
            }
            self.0.handle_drive_io_request(req)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_path_cannot_climb_out_of_the_folder() {
        assert_eq!(
            confine(r"\..\..\.ssh\authorized_keys"),
            r"\.ssh\authorized_keys"
        );
        assert_eq!(confine(r"\a\..\..\b"), r"\a\b");
        assert_eq!(confine("/../etc/passwd"), r"\etc\passwd");
        assert_eq!(confine(r"\.\x\.\y"), r"\x\y");
    }

    #[test]
    fn an_ordinary_path_is_left_as_it_was() {
        assert_eq!(confine(r"\report.pdf"), r"\report.pdf");
        assert_eq!(confine(r"\dir\*"), r"\dir\*");
        assert_eq!(confine(r"\my files\a..b.txt"), r"\my files\a..b.txt");
        assert_eq!(confine(r"\"), "");
        assert_eq!(confine(""), "");
    }
}
