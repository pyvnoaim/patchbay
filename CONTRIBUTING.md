# Contributing

## Setup

Node 22+ and Rust (`rustup`), plus Tauri's Linux packages on Linux (listed in
`.github/workflows/ci.yml`). No shell restart after installing Rust: `dev.mjs` finds
`~/.cargo/bin`. The first build compiles a few hundred crates and takes minutes.

```sh
npm ci
npm run dev      # the app window, against dev/patchbay.toml (a copy of the tracked sample)
npm test         # cargo test
npm run check    # prettier, cargo fmt, clippy -D warnings, what CI runs
npm run fmt      # fix formatting, before a pull request
```

## Layout

```
src-tauri/src/
  main.rs          setup, menu, the command list
  commands/        the #[tauri::command] surface, one thin file per area
  patchbay.rs      the config as data: load, defaults, jump chains, every ssh argv
  config.rs        the only code that writes a config (toml_edit, temp file + rename)
  import.rs        ~/.ssh/config and Royal TS documents in, ssh config out
  pty.rs           ssh on a pty, streamed to xterm.js
  sftp.rs          the file browser, by running sftp
  rdp.rs           RDP/VNC handoff to the system client, and ssh tunnels
  rdp_session.rs   remote desktop in a tab (IronRDP to a canvas)
  vnc_session.rs   VNC in a tab (vnc-rs to the same canvas)
  h264.rs          H.264 by the OS decoder (VideoToolbox, Media Foundation)
  clipboard.rs     RDP clipboard (CLIPRDR)
  sound.rs         the desktop's sound, played by the window
  keychain.rs      a remembered RDP or VNC password, in the OS keychain
  licence.rs       the company licence a shared list needs, checked offline
  hostkeys.rs      host keys the team has seen, beside the shared list
  presence.rs      who has a session on which team device, with no server
  webext.rs        Bitwarden's extension in web tabs (macOS 15.4+)
  terminal.rs      the system terminal and the local login shell, per platform
ui/                classic scripts in one global scope, no bundler
  core.js → browse.js → sessions.js → edit.js → boot.js, in that order
  theme.js         runs from <head> before first paint
  gen/, vendor/    generated, never edit by hand
site/              the website
scripts/           one-job node scripts via npm, dev.mjs is the entry point
dev/               the sample config and local scratch (your copy is ignored)
```

## Conventions

Comments say why, not what. Prefer deleting to adding. Errors are lowercase fragments
quoting the input (`no jack named "web"`). Nothing in a config file is executed as
written. The full rules and their reasons are in [CLAUDE.md](CLAUDE.md).

## Releasing

`npm run bump 0.1.0`, commit `release 0.1.0`, push `main`. No tag. CI builds macOS,
Windows and Linux into a draft release, installs each, and publishes once every leg
passes, which is when installed copies offer the update. The notes are `CHANGELOG.md`'s
`## Unreleased` section, written for someone about to install.
