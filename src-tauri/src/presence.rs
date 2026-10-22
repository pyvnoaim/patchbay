//! Who has a session open on which team device, without a server: while one is open,
//! this copy keeps a small file in a folder beside the shared list, and everyone else's
//! copy reads the folder. Part of being on a shared list, with no switch: a list where
//! some people show up and others don't answers "is anyone on this box" wrongly. Only
//! ever about the team's devices; nothing of your own is written anywhere.

use crate::patchbay;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How often the file is written again, and how long one may go unchanged before it is
/// a copy that crashed or lost the share rather than a session.
const BEAT: Duration = Duration::from_secs(60);
const FRESH: Duration = Duration::from_secs(150);
const DIR: &str = ".patchbay-presence";

/// Beside the list, and only while the list itself is there: a missing one is a share
/// that is away, and nothing is made where it will mount.
fn dir() -> Option<PathBuf> {
    let list = patchbay::list()?;
    list.is_file().then(|| list.with_file_name(DIR))
}

/// One printable line, short enough to sit in a tooltip.
fn tidy(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Who this is, as a colleague would recognise it: the account and the machine.
fn me() -> String {
    static ME: OnceLock<String> = OnceLock::new();
    ME.get_or_init(|| {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "someone".into());
        let machine = std::process::Command::new("hostname")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|m| !m.is_empty());
        tidy(&match machine {
            Some(m) => format!("{user} on {}", m.split('.').next().unwrap_or(&m)),
            None => user,
        })
    })
    .clone()
}

/// One name per account and machine, kept for good: single-instance means one copy per
/// machine, and a file deleted on a synced share goes to its bin (Google Drive's shows
/// in Finder's), so a name with the pid in it left one there every run.
fn my_file(dir: &std::path::Path) -> PathBuf {
    let safe: String = me()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    dir.join(safe)
}

/// The file's text: who, a number that moves with every write, then a device per line.
/// The number is the heartbeat. Not the file's time: clocks disagree between machines,
/// and a sync client keeps the time of the machine a file came from.
fn written(who: &str, beat: u64, names: &[String]) -> String {
    let mut out = format!("{who}\n{beat}\n");
    for n in names.iter().filter(|n| !n.starts_with(patchbay::MINE)) {
        out.push_str(&tidy(n));
        out.push('\n');
    }
    out
}

/// What a colleague's file says. Somebody else wrote it, so it is cut to size and
/// stripped of anything that isn't text before it gets near the window.
fn read(src: &str) -> Option<(String, u64, Vec<String>)> {
    let mut lines = src.lines().map(tidy).filter(|l| !l.is_empty());
    let who = lines.next()?;
    let beat = lines.next()?.parse().ok()?;
    let names: Vec<String> = lines.take(200).collect();
    (!names.is_empty()).then_some((who, beat, names))
}

/// What this copy last said, and where, so the heartbeat and the exit need not ask the
/// share where the folder is.
static SAID: Mutex<Option<(PathBuf, Vec<String>)>> = Mutex::new(None);
static BEATS: AtomicU64 = AtomicU64::new(0);

fn write_now() {
    let said = SAID.lock().unwrap().clone();
    if let Some((file, names)) = said {
        let text = written(&me(), BEATS.fetch_add(1, Relaxed), &names);
        let _ = write_own(&file, &text);
    }
}

/// Write this copy's file in place, never through a link. The folder is anyone's on the
/// share and the name is predictable, so a colleague could put a symlink there that
/// points into this machine's home, and `fs::write` would follow it. Checked before the
/// open, so a dangling link creates nothing, and on unix again after it, so a link
/// swapped in between is caught before the truncate. In place rather than temp + rename:
/// a file replaced on a synced share is one more in its bin, which is what this file
/// stopped doing. The folder is checked the same way: linked, it puts this file, a line
/// per device key the list's author chose, wherever it points - `~/.ssh/config.d` reads
/// that as ssh config.
///
/// ponytail: a swap landing between a check and the open can still create an empty file
/// through the link, never fill one; `openat` with `O_NOFOLLOW` closes that if it matters.
fn write_own(file: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let refused = || std::io::Error::other("not a plain file");
    let linked = |p: &std::path::Path| {
        p.parent()
            .is_none_or(|d| std::fs::symlink_metadata(d).map_or(true, |m| !m.is_dir()))
    };
    if linked(file) {
        return Err(refused());
    }
    let before = match std::fs::symlink_metadata(file) {
        Ok(m) if !m.is_file() => return Err(refused()),
        other => other.ok(),
    };
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        // Only once the file is known to be ours: `set_len` below.
        .truncate(false)
        .open(file)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let now = std::fs::symlink_metadata(file)?;
        let opened = f.metadata()?;
        let moved = before.is_some_and(|b| b.ino() != now.ino());
        if linked(file)
            || !now.is_file()
            || moved
            || (now.dev(), now.ino()) != (opened.dev(), opened.ino())
        {
            return Err(refused());
        }
    }
    #[cfg(not(unix))]
    let _ = before;
    f.set_len(0)?;
    f.write_all(text.as_bytes())
}

/// Say which team devices this copy has a session on, or take the file away when it
/// has none. From then on a thread here writes it again every minute: a window in the
/// background has its timers slowed, and a session is just as open behind another app.
pub fn set(names: &[String]) {
    let team: Vec<String> = names
        .iter()
        .filter(|n| !n.starts_with(patchbay::MINE))
        .cloned()
        .collect();
    if team.is_empty() {
        return clear();
    }
    let Some(dir) = dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    *SAID.lock().unwrap() = Some((my_file(&dir), team));
    write_now();
    static BEATING: OnceLock<()> = OnceLock::new();
    BEATING.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(BEAT);
            write_now();
        });
    });
}

/// Called with no sessions left, and on exit so a closed window is not "connected" for
/// another two minutes. The file is emptied of devices rather than removed, which
/// `read` takes as nobody, so nothing lands in a sync client's bin. Nothing is asked of
/// the share unless a file was written, and then only for a moment: a share that went
/// away must not hold the window open.
pub fn clear() {
    let Some((file, _)) = SAID.lock().unwrap().take() else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let text = written(&me(), BEATS.fetch_add(1, Relaxed), &[]);
    std::thread::spawn(move || {
        let _ = write_own(&file, &text);
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(1));
}

#[derive(serde::Serialize)]
pub struct Seen {
    who: String,
    names: Vec<String>,
}

/// Everyone else with a session open right now: every file whose number has moved
/// within the last couple of minutes of *this* machine's clock. A file seen for the
/// first time counts as fresh, so a copy that crashed is shown for that long after
/// this one starts, and no longer.
pub fn others() -> Vec<Seen> {
    static LAST: Mutex<Option<HashMap<PathBuf, (u64, Instant)>>> = Mutex::new(None);
    let Some(dir) = dir() else {
        return vec![];
    };
    let mine = my_file(&dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut last = LAST.lock().unwrap();
    let last = last.get_or_insert_with(HashMap::new);
    entries
        .flatten()
        .filter(|e| e.path() != mine)
        // A colleague's file is a few lines; anything bigger is not one.
        .filter(|e| e.metadata().is_ok_and(|m| m.len() < 8192))
        .filter_map(|e| {
            let (who, beat, names) = read(&std::fs::read_to_string(e.path()).ok()?)?;
            let seen = last.entry(e.path()).or_insert((beat, Instant::now()));
            if seen.0 != beat {
                *seen = (beat, Instant::now());
            }
            (seen.1.elapsed() < FRESH).then_some(Seen { who, names })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_teams_devices_are_ever_written() {
        let names = ["web".to_string(), "me:nas".into(), "db".into()];
        assert_eq!(
            written("maya on mbp", 7, &names),
            "maya on mbp\n7\nweb\ndb\n"
        );
    }

    #[test]
    fn a_colleagues_file_is_read_as_text_and_nothing_else() {
        let (who, beat, names) = read("maya on mbp\n12\nweb\n\ndb\x1b[2J\n").unwrap();
        assert_eq!((who.as_str(), beat), ("maya on mbp", 12));
        assert_eq!(names, ["web", "db[2J"]);
        // Nobody named, no heartbeat, or nothing open, is nobody.
        assert!(read("").is_none());
        assert!(read("maya\n3\n").is_none());
        assert!(read("maya\nweb\ndb\n").is_none());
        let long = "x".repeat(500);
        assert_eq!(read(&format!("{long}\n1\nweb\n")).unwrap().0.len(), 64);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_planted_where_my_file_goes_is_never_written_through() {
        let dir = std::env::temp_dir().join(format!("patchbay-presence-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("me-on-mbp");
        let precious = dir.join("authorized_keys");
        std::fs::write(&precious, "ssh-ed25519 AAAA\n").unwrap();
        std::os::unix::fs::symlink(&precious, &mine).unwrap();
        assert!(write_own(&mine, "me\n1\nweb\n").is_err());
        assert_eq!(
            std::fs::read_to_string(&precious).unwrap(),
            "ssh-ed25519 AAAA\n"
        );
        // Nor one pointing at nothing, which an open would have created.
        let gone = dir.join("LaunchAgent.plist");
        std::fs::remove_file(&mine).unwrap();
        std::os::unix::fs::symlink(&gone, &mine).unwrap();
        assert!(write_own(&mine, "x").is_err());
        assert!(!gone.exists());
        // A plain file is rewritten in place, shorter text and all.
        std::fs::remove_file(&mine).unwrap();
        write_own(&mine, "me\n1\nweb\ndb\n").unwrap();
        write_own(&mine, "me\n2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "me\n2\n");
        // A linked folder is the same trick one level up.
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let folder = dir.join(".patchbay-presence");
        std::os::unix::fs::symlink(&elsewhere, &folder).unwrap();
        assert!(write_own(&folder.join("me-on-mbp"), "x").is_err());
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
