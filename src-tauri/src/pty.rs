//! ssh on a real pseudo-terminal, streamed to xterm.js in the window. The system
//! `ssh` is spawned with the argv `patchbay.rs` builds, so the agent, `~/.ssh/config`
//! and password or host-key prompts all keep working.

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};

/// The most one message to the window carries; a backlog past it goes as several.
const BATCH: usize = 256 * 1024;
/// The least time between two messages while output keeps coming.
const GAP: std::time::Duration = std::time::Duration::from_millis(4);

pub struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

/// Spawn `program` on a pty, pump its output to `on_data` until it closes, then hand
/// the exit code to `on_exit`. Free of Tauri so it can be tested.
///
/// Bytes, not text: a UTF-8 sequence can straddle two reads, and only xterm's decoder
/// sees both halves. Output after a quiet spell goes at once, so a keystroke's echo
/// waits for nothing; while it keeps coming it is held to one message per `GAP`,
/// because a line-buffered program is one read per line and `seq 1 300000` would
/// otherwise be 180,000 messages to the window.
pub fn spawn(
    program: &str,
    args: &[String],
    cols: u16,
    rows: u16,
    on_data: impl Fn(Vec<u8>) + Send + 'static,
    on_exit: impl FnOnce(u32) + Send + 'static,
) -> Result<Session, String> {
    let pair = NativePtySystem::default()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not open a pty: {e}"))?;

    let mut cmd = CommandBuilder::new(program);
    for a in args {
        cmd.arg(a);
    }
    if let Some(dir) = dirs::home_dir() {
        cmd.cwd(dir);
    }
    // Without this ssh assumes a dumb terminal and full-screen programs misbehave.
    cmd.env("TERM", "xterm-256color");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("could not start {program}: {e}"))?;
    // The slave fd has to go, or the reader below never sees EOF.
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not read the pty: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("could not write to the pty: {e}"))?;

    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let read = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
        child.wait().map(|s| s.exit_code()).unwrap_or(1)
    });
    std::thread::spawn(move || {
        let mut due = std::time::Instant::now();
        while let Ok(mut batch) = rx.recv() {
            while batch.len() < BATCH {
                let wait = due.saturating_duration_since(std::time::Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(more) => batch.extend_from_slice(&more),
                    Err(_) => break,
                }
            }
            on_data(batch);
            due = std::time::Instant::now() + GAP;
        }
        on_exit(read.join().unwrap_or(1));
    });

    Ok(Session {
        writer,
        master: pair.master,
    })
}

/// Owner-only: a session log holds whatever the remote box printed.
#[cfg(unix)]
fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

#[derive(Default)]
pub struct Sessions(Mutex<HashMap<u32, Session>>);

impl Sessions {
    /// Output goes down `out` as raw bytes, then the exit status as a number: one
    /// channel, so the status can't overtake the last of the output.
    /// `log` appends the same bytes to a file, when session logging is turned on.
    pub fn open(
        &self,
        out: Channel<InvokeResponseBody>,
        id: u32,
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        log: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        let exit = out.clone();
        let log_file = log.and_then(|path| {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            open_log(&path).ok()
        });
        let log_file = log_file.map(Mutex::new);
        let session = spawn(
            program,
            args,
            cols,
            rows,
            move |chunk| {
                if let Some(f) = &log_file {
                    let _ = f.lock().unwrap().write_all(&chunk);
                }
                let _ = out.send(InvokeResponseBody::Raw(chunk));
            },
            move |code| {
                let _ = exit.send(InvokeResponseBody::Json(code.to_string()));
            },
        )?;
        self.0.lock().unwrap().insert(id, session);
        Ok(())
    }

    pub fn write(&self, id: u32, data: &str) -> Result<(), String> {
        let mut map = self.0.lock().unwrap();
        let s = map.get_mut(&id).ok_or("that session is already closed")?;
        s.writer
            .write_all(data.as_bytes())
            .and_then(|_| s.writer.flush())
            .map_err(|e| e.to_string())
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let map = self.0.lock().unwrap();
        let Some(s) = map.get(&id) else { return Ok(()) };
        s.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())
    }

    /// Dropping the session closes the master fd, which hangs up ssh.
    pub fn close(&self, id: u32) {
        self.0.lock().unwrap().remove(&id);
    }
}

pub type Shared = Arc<Sessions>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn output_reaches_the_callback_and_the_exit_code_lands() {
        let (tx, rx) = channel();
        let (etx, erx) = channel();
        let _s = spawn(
            "echo",
            &["patchbay-pty-works".to_string()],
            80,
            24,
            move |chunk| {
                let _ = tx.send(chunk);
            },
            move |code| {
                let _ = etx.send(code);
            },
        )
        .unwrap();

        let out = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no pty output arrived");
        let out = String::from_utf8_lossy(&out);
        assert!(out.contains("patchbay-pty-works"), "got {out:?}");
        assert_eq!(erx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    }

    #[test]
    fn input_written_to_the_pty_comes_back_out() {
        let (tx, rx) = channel();
        let s = spawn(
            "cat",
            &[],
            80,
            24,
            move |c| {
                let _ = tx.send(c);
            },
            |_| {},
        )
        .unwrap();
        let sessions = Sessions::default();
        sessions.0.lock().unwrap().insert(7, s);

        sessions.write(7, "ping\n").unwrap();
        // A pty is a stream: the echo can arrive in pieces, so gather until it is whole.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut out = String::new();
        while !out.contains("ping") && std::time::Instant::now() < deadline {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                out.push_str(&String::from_utf8_lossy(&chunk));
            }
        }
        assert!(out.contains("ping"), "cat echoed {out:?}");
    }

    #[test]
    fn multibyte_text_survives_being_split_across_reads() {
        let (tx, rx) = channel();
        let (etx, erx) = channel();
        // 3 bytes each and far past one read, so some sequence lands on a boundary.
        let _s = spawn(
            "sh",
            &[
                "-c".into(),
                "for i in $(seq 4000); do printf '€'; done".into(),
            ],
            80,
            24,
            move |c| {
                let _ = tx.send(c);
            },
            move |code| {
                let _ = etx.send(code);
            },
        )
        .unwrap();
        erx.recv_timeout(Duration::from_secs(10)).unwrap();
        let bytes: Vec<u8> = rx.try_iter().flatten().collect();
        let text = String::from_utf8(bytes).expect("a sequence was cut");
        assert_eq!(text.matches('€').count(), 4000);
    }

    #[test]
    fn a_flood_of_short_lines_goes_as_few_messages() {
        let (tx, rx) = channel();
        let (etx, erx) = channel();
        let _s = spawn(
            "seq",
            &["1".into(), "100000".into()],
            80,
            24,
            move |c: Vec<u8>| {
                let _ = tx.send(c.len());
            },
            move |code| {
                let _ = etx.send(code);
            },
        )
        .unwrap();
        erx.recv_timeout(Duration::from_secs(10)).unwrap();
        let sizes: Vec<usize> = rx.try_iter().collect();
        assert!(sizes.iter().sum::<usize>() > 500_000);
        assert!(sizes.len() < 1000, "{} messages", sizes.len());
    }

    #[test]
    fn writing_to_a_closed_session_says_so() {
        let sessions = Sessions::default();
        assert!(sessions
            .write(99, "x")
            .unwrap_err()
            .contains("already closed"));
    }
}
