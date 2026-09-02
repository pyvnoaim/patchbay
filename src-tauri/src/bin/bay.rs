//! **A port of `src/cli.ts`** - the CLI half of patchbay, as a binary rather than a
//! Node script, so installing it is the same download as the window. Arg dispatch and
//! process spawning only; everything worth testing lives in `patchbay.rs`.
//!
//! The modules come in by path because this is a second bin of the app crate, not a
//! library: the window can't be a dependency of the CLI, and the logic is one file.

// Both modules carry more than the CLI reaches - the window uses the rest.
#[allow(dead_code)]
#[path = "../import.rs"]
mod import;
#[allow(dead_code)]
#[path = "../patchbay.rs"]
mod patchbay;

use patchbay::{Jack, Jacks};
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::{exit, Command, Stdio};

const TEMPLATE: &str = r#"# patchbay - every host, one jack away
# Anything here is inherited by every jack below.
[defaults]
user = "root"

[jack.example]
host = "192.0.2.10"
folders = ["demo"]
desc = "delete me"

# [jack.prod-web]
# host = "10.0.0.4"
# user = "deploy"
# key  = "~/.ssh/prod"
# jump = "bastion"              # another jack name, or a raw user@host
# forward = ["8080:localhost:80"]
"#;

/// Names come from `bay ls --names`, so a snippet never goes stale with the config.
const COMPLETIONS: [(&str, &str); 3] = [
    (
        "zsh",
        "# eval \"$(bay completion zsh)\" in ~/.zshrc\n\
         _bay() { compadd -- ${(f)\"$(bay ls --names)\"} }\n\
         compdef _bay bay",
    ),
    (
        "bash",
        "# eval \"$(bay completion bash)\" in ~/.bashrc\n\
         _bay() { COMPREPLY=($(compgen -W \"$(bay ls --names)\" -- \"$2\")); }\n\
         complete -F _bay bay",
    ),
    (
        "fish",
        "# bay completion fish > ~/.config/fish/completions/bay.fish\n\
         complete -c bay -f -a \"(bay ls --names)\"",
    ),
];

const WIN: bool = cfg!(target_os = "windows");

fn c(code: &str, s: &str) -> String {
    match std::io::stdout().is_terminal() {
        true => format!("\x1b[{code}m{s}\x1b[0m"),
        false => s.to_string(),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("{}", c("31", msg));
    exit(1)
}

/// Name, host, folders and - once you have more than your own list - the space.
fn row(name: &str, j: &Jack, w: usize) -> String {
    let folders = match j.folders.as_deref() {
        Some(f) if !f.is_empty() => format!(" [{}]", f.join(" ")),
        _ => String::new(),
    };
    let space = match &j.space {
        Some(sp) => format!(" @{sp}"),
        None => String::new(),
    };
    format!("{name:w$}  {}{folders}{space}", j.host)
}

fn select<'a>(jacks: &'a Jacks, filter: Option<&str>, space: Option<&str>) -> Vec<(&'a String, &'a Jack)> {
    let rows: Vec<_> = jacks
        .iter()
        .filter(|(name, j)| patchbay::matches(j, name, filter))
        .filter(|(_, j)| space.is_none_or(|sp| j.space.as_deref().unwrap_or("") == sp))
        .collect();
    if !rows.is_empty() {
        return rows;
    }
    match (filter, space) {
        (Some(f), _) => die(&format!("nothing matches \"{f}\"")),
        (None, Some(sp)) => die(&format!("nothing in space \"{sp}\"")),
        (None, None) => die("no jacks configured"),
    }
}

fn list(jacks: &Jacks, filter: Option<&str>, names: bool, space: Option<&str>) {
    let rows = select(jacks, filter, space);
    // One name per line, for the completion snippets and anything else piping this.
    if names {
        for (name, _) in rows {
            println!("{name}");
        }
        return;
    }
    let w = rows.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
    for (name, j) in rows {
        let folders = match j.folders.as_deref() {
            Some(f) if !f.is_empty() => c("2", &format!(" [{}]", f.join(" "))),
            _ => String::new(),
        };
        let space = match &j.space {
            Some(sp) => c("2", &format!(" @{sp}")),
            None => String::new(),
        };
        println!("{}  {}{folders}{space}", c("33", &format!("{name:w$}")), c("2", &j.host));
    }
}

/// fzf if it's there, plain list if it isn't. ponytail: no bundled picker, add one when fzf annoys you.
fn pick(jacks: &Jacks) -> Option<String> {
    let w = jacks.keys().map(|n| n.chars().count()).max().unwrap_or(0);
    let lines: Vec<String> = jacks.iter().map(|(name, j)| row(name, j, w)).collect();
    // The columns are padded then joined by two spaces, so a name is field 1 - which is
    // what the preview and the returned line are read back as.
    // $BAY rather than a path we quote ourselves: fzf runs the preview through a shell.
    let mut child = Command::new("fzf")
        .args([
            "--height=40%", "--reverse", "--prompt=jack> ",
            "--delimiter", "  ",
            "--preview", r#""$BAY" {1} -n"#, "--preview-window", "down,3",
        ])
        .env("BAY", std::env::current_exe().ok()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(lines.join("\n").as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let picked = line.trim().split("  ").next().unwrap_or("").trim().to_string();
    (!picked.is_empty()).then_some(picked)
}

fn edit(space: Option<&str>) -> ! {
    let path = patchbay::space_path(&patchbay::config_path(), space);
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, TEMPLATE) {
            die(&format!("{}: {e}", path.display()));
        }
        eprintln!("{}", c("2", &format!("created {}", path.display())));
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| if WIN { "notepad".into() } else { "vi".into() });
    // Windows editors are usually .cmd shims (code, subl), which spawn refuses without a shell.
    let mut cmd = match WIN {
        true => {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(&editor).arg(&path);
            c
        }
        false => {
            let mut c = Command::new(&editor);
            c.arg(&path);
            c
        }
    };
    match cmd.status() {
        Ok(s) => exit(s.code().unwrap_or(0)),
        Err(_) => die(&format!("could not run \"{editor}\"")),
    }
}

fn import_config(file: Option<&str>) -> ! {
    let path = match file {
        Some(f) => std::path::PathBuf::from(f),
        None => match dirs::home_dir() {
            Some(h) => h.join(".ssh").join("config"),
            None => die("no home directory"),
        },
    };
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| die(&format!("no ssh config at {}", path.display())));
    let found = import::from_ssh_config(&src);
    for w in &found.warnings {
        eprintln!("{}", c("33", w));
    }
    if found.hosts.is_empty() {
        die(&format!("no hosts in {}", path.display()));
    }
    print!("{}", import::to_toml(&found.hosts));
    exit(0)
}

fn load(path: &Path) -> Jacks {
    if !path.exists() {
        die(&format!("no config at {} - run `bay edit` to start one", path.display()));
    }
    patchbay::load_all(path).unwrap_or_else(|e| die(&format!("{}: {e}", path.display())))
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cmd = argv.first().map(String::as_str);
    let rest = argv.get(1..).unwrap_or(&[]);

    if let Some("-h" | "--help") = cmd {
        println!(
            "bay              pick a jack (fzf) or list them
bay <name>       connect - substring is enough, and any other flag goes to ssh
bay <name> -n    print the ssh command instead of running it
bay <name> -- <cmd>   run a command instead of a shell
bay ls [filter]  list jacks, filtered by name or folder
                 --names for names alone, --space <space> for one space
bay edit [space] open {}, or spaces/<space>.toml
bay import [file]  print TOML for the hosts in your ssh config
bay completion <shell>   a snippet for zsh, bash or fish
bay --version",
            patchbay::config_path().display()
        );
        exit(0);
    }
    if cmd == Some("--version") {
        println!("patchbay {}", env!("CARGO_PKG_VERSION"));
        exit(0);
    }
    if cmd == Some("completion") {
        let want = rest.first().map(String::as_str).unwrap_or("");
        match COMPLETIONS.iter().find(|(shell, _)| *shell == want) {
            Some((_, snippet)) => println!("{snippet}"),
            None => die(&format!("no completion for \"{want}\" - zsh, bash or fish")),
        }
        exit(0);
    }
    if cmd == Some("edit") {
        edit(rest.first().map(String::as_str));
    }
    // Stdout, not the config file: config.rs is the only thing that edits a patchbay.toml,
    // and printing means you read it before you keep it.
    if cmd == Some("import") {
        import_config(rest.first().map(String::as_str));
    }

    let path = patchbay::config_path();
    let jacks = load(&path);

    if cmd == Some("ls") {
        let si = rest.iter().position(|a| a == "--space");
        let space = si.and_then(|i| rest.get(i + 1)).map(String::as_str);
        if si.is_some() && space.is_none() {
            die("--space needs a space name");
        }
        let filter = rest
            .iter()
            .enumerate()
            .find(|(i, a)| !a.starts_with("--") && Some(*i) != si.map(|s| s + 1))
            .map(|(_, a)| a.as_str());
        list(&jacks, filter, rest.iter().any(|a| a == "--names"), space);
        exit(0);
    }

    let target = match cmd {
        Some(t) => t.to_string(),
        None => match pick(&jacks) {
            Some(t) => t,
            None => {
                list(&jacks, None, false, None);
                exit(0)
            }
        },
    };

    let name = patchbay::resolve(&target, &jacks).unwrap_or_else(|e| die(&e));
    let j = &jacks[&name];
    // `ssh = false` is the device saying so; `primary` only names the default action, so a
    // box with a web ui *and* ssh still connects. Without this it built an ssh command for
    // an appliance that never listened on 22.
    if j.ssh == Some(false) {
        let what = match patchbay::primary(j).as_str() {
            "web" => format!(" - it's a web ui at {}", j.url.as_deref().unwrap_or("")),
            "rdp" => format!(" - it's remote desktop on port {}", j.rdp.unwrap_or(0)),
            "vnc" => format!(" - it's vnc on port {}", j.vnc.unwrap_or(0)),
            _ => String::new(),
        };
        die(&format!("\"{name}\" isn't reached by ssh{what}"));
    }

    let mut args = patchbay::ssh_args(&name, &jacks).unwrap_or_else(|e| die(&e));
    let dash = rest.iter().position(|a| a == "--");
    let flags = match dash {
        Some(d) => &rest[..d],
        None => rest,
    };
    let dry = flags.iter().any(|f| f == "-n" || f == "--dry-run");
    // Everything else is ssh's, and ssh wants its options before the target - which
    // ssh_args puts last. Dropping them silently is how `bay web -v` stopped being verbose.
    let spec = args.pop().unwrap_or_default();
    args.extend(flags.iter().filter(|f| *f != "-n" && *f != "--dry-run").cloned());
    args.push(spec);
    if let Some(d) = dash {
        args.extend_from_slice(&rest[d + 1..]);
    }

    if dry {
        println!("ssh {}", args.join(" "));
        exit(0);
    }

    match Command::new("ssh").args(&args).status() {
        Ok(s) => exit(s.code().unwrap_or(1)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => die(match WIN {
            true => "no ssh on PATH - enable the OpenSSH Client feature in Windows Settings",
            false => "no ssh on PATH",
        }),
        Err(e) => die(&format!("ssh: {e}")),
    }
}
