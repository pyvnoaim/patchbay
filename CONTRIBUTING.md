# Contributing

## Setup

You need Node 22+ and Rust (`rustup`), plus the platform packages Tauri needs on
Linux (see `.github/workflows/test.yml` for the list).

```sh
npm ci
npm run dev      # the app window, against dev/patchbay.toml (a copy of the tracked sample)
npm test         # cargo test, through scripts/dev.mjs so PATH and the config path are right
npm run check    # prettier --check, cargo fmt --check, cargo clippy -D warnings
npm run fmt      # prettier --write and cargo fmt
```

`npm run check` is what CI runs. Run `npm run fmt` before opening a pull request.

## Layout

```
src-tauri/src/
  main.rs          Tauri setup: plugins, menu, the command list
  commands/        the #[tauri::command] surface, one file per area, kept thin
  patchbay.rs      the config as data: load, defaults, jump chains, every ssh argv
  config.rs        the only code that writes a config (toml_edit, temp file + rename)
  import.rs        readers for ~/.ssh/config and Royal TS documents
  pty.rs           ssh on a pty, streamed to xterm.js
  sftp.rs          the file browser, by running /usr/bin/sftp
  rdp.rs           remote desktop by handoff (.rdp file) and the ssh tunnels
  rdp_session.rs   remote desktop in a tab (IronRDP to a canvas)
  clipboard.rs     the CLIPRDR backend for rdp_session.rs
  terminal.rs      opening the system terminal, per platform
ui/                the window: classic scripts sharing one global scope, no bundler
  core.js → browse.js → sessions.js → edit.js → boot.js, loaded in that order
  theme.js         runs from <head> before first paint
  gen/, vendor/    generated; never edit by hand
site/              the website
scripts/           one-job node scripts, run via npm; dev.mjs is the entry point
dev/               the sample config and local scratch (your own copy is ignored)
```

## Conventions

- **Comments say why, not what.** One to three lines. If a line needs a comment to
  say what it does, rename something. Mark a deliberate shortcut `ponytail:` with its
  ceiling and the upgrade path.
- **Prefer deleting to adding.** No interface with one implementation, no config for a
  value that never changes, nothing built for later.
- **Errors are lowercase fragments naming what failed, quoting the input:**
  `no jack named "web"`. They are shown in the window, so write them for a person.
- **Rust:** `Result<T, String>` at the command boundary. Pure logic goes in
  `patchbay.rs` with a test next to it; commands stay thin. Platform code is
  `#[cfg(target_os = ...)]`, never a runtime flag. Every config writer has a
  `*_at(path, …)` twin the tests drive against a scratch file.
- **Frontend:** no `import`, no inline `<script>` (the CSP is `script-src 'self'`).
  Everything interpolated into `innerHTML` goes through `esc()`. Colours only from the
  `:root` custom properties, each with a light-mode value. Icons via `icon("name")`,
  added to `USED` in `scripts/icons.mjs` and regenerated with `npm run icons`.
- **Nothing in a config file is executed as written.** The most it can produce is an
  `ssh` argv, and `patchbay.rs` refuses anything that could become an option.

The full list of non-obvious rules and the reasons behind them is in `CLAUDE.md`.

## Releasing

`npm run bump 0.1.0`, commit, tag `v0.1.0`, push the tag. CI builds every platform
into a draft release; publishing the draft is the moment installed copies start
offering the update. Release notes are the `## Unreleased` section of `CHANGELOG.md`,
written for someone about to install.
