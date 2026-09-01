//! Hand the ssh command to the system terminal. Same rule CLAUDE.md sets for RDP:
//! we never embed the client, we hand off to the one the OS already has.

use std::process::Command;

fn is_bare(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%_+=:,./-".contains(c))
}

/// Quote for a POSIX shell — the command goes through `do script` / `-e "…"`.
#[cfg(not(windows))]
fn quote(s: &str) -> String {
    if is_bare(s) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// cmd.exe has no single quotes; a path with spaces needs double ones.
#[cfg(windows)]
fn quote(s: &str) -> String {
    if is_bare(s) {
        return s.to_string();
    }
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// The command as a user would type it — shown in the UI and used by the
/// AppleScript/`sh -c` handoff, so it follows the host platform's quoting.
pub fn command_line(args: &[String]) -> String {
    command_line_of("ssh", args)
}

pub fn command_line_of(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|a| quote(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "macos")]
pub fn open(args: &[String]) -> Result<(), String> {
    let cmd = command_line(args);
    // Terminal.app unless iTerm is what's actually installed.
    // `launch` starts the app WITHOUT its normal "open a window" startup, so the
    // command below supplies the only window. Using `activate` first opens an empty
    // one as well, and you get two. Activate last, once the window exists.
    let script = if std::path::Path::new("/Applications/iTerm.app").exists() {
        format!(
            r#"tell application "iTerm"
                 launch
                 create window with default profile command "{}"
                 activate
               end tell"#,
            escape(&cmd)
        )
    } else {
        format!(
            r#"tell application "Terminal"
                 launch
                 do script "{}"
                 activate
               end tell"#,
            escape(&cmd)
        )
    };
    Command::new("osascript")
        .args(["-e", &script])
        .spawn()
        .map_err(|e| format!("could not open a terminal: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "windows")]
pub fn open(args: &[String]) -> Result<(), String> {
    // Windows Terminal if it's there (it ships with Win11), else a console window.
    if Command::new("wt.exe").arg("ssh").args(args).spawn().is_ok() {
        return Ok(());
    }
    Command::new("cmd")
        .args(["/c", "start", "", "ssh"])
        .args(args)
        .spawn()
        .map_err(|e| format!("could not open a terminal: {e}"))?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn open(args: &[String]) -> Result<(), String> {
    let cmd = command_line(args);
    // ponytail: first terminal that launches wins. Add $TERMINAL support when someone's isn't here.
    let candidates: [(&str, &[&str]); 7] = [
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("alacritty", &["-e"]),
        ("kitty", &[]),
        ("foot", &[]),
        ("xterm", &["-e"]),
    ];
    for (bin, flags) in candidates {
        let ok = Command::new(bin)
            .args(flags)
            .args(["sh", "-c", cmd.as_str()])
            .spawn()
            .is_ok();
        if ok {
            return Ok(());
        }
    }
    Err("no terminal emulator found — tried gnome-terminal, konsole, alacritty, kitty, foot, xterm".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_only_what_needs_it() {
        let args = ["-J".into(), "deploy@10.0.0.4".into(), "root@10.0.0.5".into()];
        assert_eq!(command_line(&args), "ssh -J deploy@10.0.0.4 root@10.0.0.5");
    }

    #[cfg(not(windows))]
    #[test]
    fn command_line_quotes_paths_with_spaces_and_quotes() {
        let args = ["-i".into(), "/Users/a b/.ssh/id".into()];
        assert_eq!(command_line(&args), "ssh -i '/Users/a b/.ssh/id'");
        assert_eq!(command_line(&["it's".into()]), r"ssh 'it'\''s'");
    }

    #[cfg(windows)]
    #[test]
    fn command_line_uses_double_quotes_on_windows() {
        let args = ["-i".into(), r"C:\Users\a b\.ssh\id".into()];
        assert_eq!(command_line(&args), r#"ssh -i "C:\Users\a b\.ssh\id""#);
    }
}
