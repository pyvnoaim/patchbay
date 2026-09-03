//! ssh running in a real pseudo-terminal, streamed to xterm.js in the window.
//!
//! We still don't reimplement ssh - this spawns the same `/usr/bin/ssh` with the
//! same argv the CLI would, just with a pty on the near end instead of the user's
//! terminal. Agent, ~/.ssh/config and known_hosts keep working, and because it's a
//! real tty, password and host-key prompts do too.

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
}

/// Spawns `program` on a pty and pumps its output to `on_data` until it closes,
/// then hands the exit code to `on_exit`. Kept free of Tauri so it can be tested.
pub fn spawn(
    program: &str,
    args: &[String],
    cols: u16,
    rows: u16,
    on_data: impl Fn(String) + Send + 'static,
    on_exit: impl FnOnce(u32) + Send + 'static,
) -> Result<Session, String> {
    let pair = NativePtySystem::default()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
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

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                // Lossy is right here: a UTF-8 sequence can straddle two reads,
                // and xterm re-joins the pieces on its side anyway.
                Ok(n) => on_data(String::from_utf8_lossy(&buf[..n]).to_string()),
            }
        }
        on_exit(child.wait().map(|s| s.exit_code()).unwrap_or(1));
    });

    Ok(Session { writer, master: pair.master })
}

/// A session log is a transcript of whatever ran, which can hold anything the
/// remote box printed - owner-only, the same reasoning as `team.toml`.
#[cfg(unix)]
fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)
}

#[cfg(not(unix))]
fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().create(true).append(true).open(path)
}

#[derive(Default)]
pub struct Sessions(Mutex<HashMap<u32, Session>>);

impl Sessions {
    /// Output is emitted as `pty:<id>`; on exit, `pty-exit:<id>` carries the status.
    /// `log` appends the same bytes to a file, when session logging is turned on.
    pub fn open(
        &self,
        app: &AppHandle,
        id: u32,
        program: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        log: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        let data_handle = app.clone();
        let exit_handle = app.clone();
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
                    let _ = f.lock().unwrap().write_all(chunk.as_bytes());
                }
                let _ = data_handle.emit(&format!("pty:{id}"), chunk);
            },
            move |code| {
                let _ = exit_handle.emit(&format!("pty-exit:{id}"), code);
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
            .map_err(|e| format!("{e}"))
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let map = self.0.lock().unwrap();
        let Some(s) = map.get(&id) else { return Ok(()) };
        s.master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("{e}"))
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
            move |chunk| { let _ = tx.send(chunk); },
            move |code| { let _ = etx.send(code); },
        )
        .unwrap();

        let out = rx.recv_timeout(Duration::from_secs(5)).expect("no pty output arrived");
        assert!(out.contains("patchbay-pty-works"), "got {out:?}");
        assert_eq!(erx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    }

    #[test]
    fn input_written_to_the_pty_comes_back_out() {
        let (tx, rx) = channel();
        let s = spawn("cat", &[], 80, 24, move |c| { let _ = tx.send(c); }, |_| {}).unwrap();
        let sessions = Sessions::default();
        sessions.0.lock().unwrap().insert(7, s);

        sessions.write(7, "ping\n").unwrap();
        // A pty is a stream: the first chunk can be "pin" with the rest still in
        // flight, and asserting on one read made this fail under load roughly once in
        // ten. Gather until the echo is whole, or the deadline says it never will be.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut out = String::new();
        while !out.contains("ping") && std::time::Instant::now() < deadline {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                out.push_str(&chunk);
            }
        }
        assert!(out.contains("ping"), "cat echoed {out:?}");
    }

    #[test]
    fn writing_to_a_closed_session_says_so() {
        let sessions = Sessions::default();
        assert!(sessions.write(99, "x").unwrap_err().contains("already closed"));
    }
}
