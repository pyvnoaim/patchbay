# patchbay

SSH connection manager: TOML files, exec `ssh`. A tiny answer to Royal TS: no
Electron, no sync service, no stored credentials.

Devices live in `patchbay.toml`. `folders` is a list on each device, and a folder can
carry a note. With a shared list set there are exactly two files, two vaults: **Mine**
(own config) and **Team** (shared list). A device is in one, nothing is merged. In
memory yours is `me:<key>`, the team's keeps its bare key.

**Each module's header comment (`//!` in Rust, the top `//` block in `ui/`) holds its
non-obvious whys: ordering constraints, platform quirks, decisions that look wrong
until you know the story. Read it before changing the module.** This file keeps the
map, the commands and the rules that cut across files.

## Map

- `src-tauri/src/patchbay.rs`: config load, `[defaults]`, jump-chain walk, name resolve, every ssh argv. The logic worth testing. Tests at the bottom.
- `src-tauri/src/config.rs`: the only code that _writes_ a config.
- `src-tauri/src/commands/`: the `#[tauri::command]` surface, one thin file per area. `mod.rs` holds the two-vault plumbing (`file_for`, `load_all`, stamps).
- `src-tauri/src/main.rs`: setup, macOS menu, command list, the channel interceptor (`pbchan:`).
- `pty.rs` terminals · `sftp.rs` files · `rdp.rs` RDP/VNC handoff and tunnels · `rdp_session.rs` in-window RDP (IronRDP) · `vnc_session.rs` in-window VNC (vnc-rs) · `h264.rs` OS video decode · `clipboard.rs` RDP clipboard · `sound.rs` RDP sound, played by the webview · `webext.rs` Bitwarden · `hostkeys.rs` team host keys · `presence.rs` who is on a device · `keychain.rs` remembered RDP/VNC passwords · `licence.rs` team licence · `terminal.rs` system terminal and local login shell · `import.rs` ssh config and Royal TS in, ssh config out.
- `src-tauri/capabilities/default.json`: grants `core:default`. Load-bearing, see Non-obvious.
- `src-tauri/Info.plist`: merged into the macOS bundle. Without it a plain-http device page is a silent white pane.
- Beside the config, never in it (this machine's answer, not the list's): `web_trusted` (checks waived), `rdp_known_hosts` (certs seen).
- `ui/`: no framework, no bundler. **Classic scripts in one global scope. Order in `index.html` matters**: `core.js` (state, helpers) → `browse.js` (tree, detail, map, palette) → `sessions.js` (terminal tabs) → `edit.js` (menu, sheets, config writes) → `boot.js` (keyboard, startup, the only file that _runs_). `theme.js` runs from `<head>` before first paint, since the theme is an attribute the first frame needs. Cross-file references only inside function bodies, never at top level.
- `ui/gen/` (`npm run icons` / `npm run brands`) and `ui/vendor/` (xterm, `npm run vendor`): committed, never hand-edited.
- `dev/patchbay.example.toml`: tracked sample. `dev.mjs` copies it to the gitignored `dev/patchbay.toml`. A real address in a public repo is as bad as a password.
- `CHANGELOG.md`: the release notes, written only here, for someone about to install.
- `TODO.md`: next and ideas. A line leaves when it ships (into the changelog) or is decided against. No plan files: the code says what was built.
- `scripts/`: one-job node scripts via npm, never imported by the app. `dev.mjs` is the single local entry point. `draw.mjs` is the exception: PNG writer and SDF helpers shared by `icon.mjs` and `dmg.mjs`, so neither needs an image dependency. `notes.mjs` is CI's: one version's changelog section.
- `LICENSE`: free for one person, company licence for a shared list. 0.1.58 and earlier stay [PolyForm Perimeter](https://polyformproject.org/licenses/perimeter/1.0.1). `licence.rs` asks for nothing until `ON_SALE` turns on. `TODO.md` has the steps left.

## Commands

```sh
npm run dev      # the app window, against dev/patchbay.toml
npm test         # cargo test, through dev.mjs so PATH and config path are right
npm run check    # prettier --check, cargo fmt --check, clippy -D warnings, what CI runs
npm run fmt      # prettier --write and cargo fmt
npm run build    # patchbay.app / .exe / .deb, on macOS opens the .dmg
npm run bump [0.1.0]  # version (next patch if none) in the four files that carry it
npm run icon     # app icon from scripts/icon.mjs
npm run dmg      # .dmg background from scripts/dmg.mjs (macOS, needs tiffutil)
npm run icons    # ui/gen/icons.js after editing USED in scripts/icons.mjs
npm run brands   # ui/gen/brands.js after editing scripts/brands.mjs
npm run vendor   # refresh ui/vendor/
npm run licence "Acme GmbH" 2027-10-20  # sign a team licence with ~/.tauri/patchbay-licence.key
```

## "push"

When I say **push**, follow `.claude/GATE.md`: review to release, and the _only_ time
anything is committed.

**Never commit or push unless that message asks for it.** Not after a cleanup, not
because the work looks finished, the tree is clean or the change is small. Leave the
work in the tree and say what's ready.

## Conventions

Read a neighbouring file before adding one and match it. The rules that aren't obvious:

**Everywhere**

- Comments say _why_, never _what_. If a line needs a what-comment, rename something. Worth writing: a non-obvious ordering, a platform quirk, a deliberate shortcut.
- Mark simplifications `ponytail:` with ceiling and upgrade path: `// ponytail: one thread per jack, bounded pool if someone brings a thousand`.
- Prefer deleting to adding. No interface with one implementation, no config for a constant, no scaffolding for later.
- Errors are lowercase fragments naming what failed and quoting the input: `no jack named "web"`, `jump loop through "loop2"`. Written for someone reading a sheet, not a stack trace.

**Rust (`src-tauri/src/`)**

- `Result<T, String>` at the command boundary. The string is shown to the user.
- Platform code is `#[cfg(target_os = ...)]` in `terminal.rs`, never a runtime `if`.
- Pure logic goes in `patchbay.rs` with a test there. `commands/` stays thin. A helper with its own test may sit beside its command.
- `npm run check` must pass. Format with `npm run fmt`, not by hand.

**Frontend (`ui/`)**

- No `import`. Classic scripts. `withGlobalTauri` is on: `window.__TAURI__.core.invoke`.
- CSP is `script-src 'self'`: no inline `<script>`, no CDN. Styles go in `app.css`.
- Everything interpolated into `innerHTML` goes through `esc()`, no exceptions: a jack name comes from a file a colleague may have written.
- Colours only from `:root` custom properties, each with a value in `:root[data-theme="light"]`. No hex outside `:root`.
- Corners and heights only from the `:root` scale: `--r-sm`/`--r-md`/`--r-lg`/`--r-xl`/`--r-pill` (key hint, control, surface, floating sheet, pill) and `--h-control` for every toolbar button, tab and field. Shapes under 4px and round dots keep their own.
- Icons are Lucide via `icon("name")`: add to `USED` in `scripts/icons.mjs`, `npm run icons`. No pasted SVG, no `lucide-react`.
- A jack's `os` renders through `osIcon()` (brand mark, else Lucide fallback, else `server`). Maps in `scripts/brands.mjs`, then `npm run brands`. Brand colours go through `readable()`: nine of 29 fail contrast on one theme.
- Never fetch device favicons: needs TLS (self-signed everywhere), half sit behind a bastion, and opening the window would hit every host.
- One render path: mutate state, call `render()`. No targeted DOM patching.
- Errors outside a form go through `alertish()` (a pill).
- **A new full-screen overlay needs three registrations besides `index.html`. Each failure is quiet:**
  - `OVERLAYS()` in `core.js` (feeds `modalOpen()` and the web-tab shrink observer). Missing: the webview paints over it. The context menu was missed once.
  - the `#sheetwrap, #askwrap, #importwrap` rule in `app.css`. Missing: no `position: fixed`, it stretches the layout.
  - the modal guard in `boot.js`. Missing: Escape doesn't close it and global chords fire (⌘N opens the jack sheet on top).
- **A live session owns the keyboard.** Global `keydown` returns early when `activeId !== null`. Only window chords (⌘K, ⌘N, ⌘D, ⌘W, ⌘[/]) are intercepted, and each new one is a key the shell loses. More in `sessions.js`'s header.
- Shortcut labels from `chord("k")`, matching via `chorded(e)`/`chordKey(e)`. Never a literal `⌘` or `e.metaKey || e.ctrlKey` (that made Ctrl+W in a shell a close prompt).
- The default `contextmenu` (Reload/Inspect) is suppressed. New actions go in our `contextmenu` handler _and_ a visible button.
- `localStorage` holds habits only (theme, tabs to reopen, open folders, recency), never list data.
- **Anything spawned detached is killed on exit.** A pty dies with its master fd, `ssh -N -L` doesn't: `RunEvent::Exit` calls `close_all()`, or tunnels keep their ports.
- A web tab is a child webview: an OS view above the page, ignoring our CSS, sized exactly over its host div or to nothing.
- **Core APIs need a capability. Our commands don't.** `#[tauri::command]`s in `generate_handler!` work without permission, but `listen`, `emit`, clipboard, path are denied unless `src-tauri/capabilities/*.json` grants them. Without it `invoke("jacks")` works and sessions sit mute. A `window.__TAURI__` call rejecting for no reason: check the capability first.
- **`connect-src ipc: http://ipc.localhost` is load-bearing.** Without it WebKit blocks Tauri's IPC `fetch` and Tauri silently falls back to `postMessage`: binary channel messages arrive as JSON number arrays, an RDP frame 150 ms instead of 9. A frame reaching `chan.onmessage` as anything but `ArrayBuffer` means check the CSP.
- **No channel message takes Tauri's own path.** It parks anything over 1 KB in a queue any webview can drain, ACL or not. A web tab took every frame that way. `intercept` in `main.rs` evals small messages into the window and parks big ones for `pbchan:`, which answers only the `main` webview. So `pbchan:` sits in `connect-src` too, equally quiet when missing.
- **Ours need no capability only from the local origin.** A device's web UI is an `Origin::Remote` child webview matching no `ExecutionContext`, so it reaches none of our commands. That alone stands between a Synology login page and `delete_jack`. **Never add `remote` to a capability.**

**The site (`site/`)**

- **Only mock data**: hosts `10.0.0.x`, customer `Acme GmbH`, document `acme.rtsz`. A real address or customer name has leaked twice.
- Line-breaking rules and why animations live in `script.js`: top of `site/style.css`.

**Validating what reaches the outside**

- Anything reaching a `style` attribute or the desktop opener is checked in Rust on save and in JS before use: `[colors]` must be `#rrggbb`, a jack's `url` http(s).

**Writing the config**

- **A snippet is typed, never run.** Never add a way to press Enter: `snippet_ok()` refuses every control character (save, read, window), which is why a colleague's file can put text in front of you but not run it. More in `sessions.js`'s header.
- **Nothing in a config file is executed as written.** The most it produces is an `ssh` argv. `dest()` in `patchbay.rs` enforces it: a `host` or raw `jump` starting with `-` is an ssh _option_, and `task_argv` appends `traceroute <host>`, the operand a smuggled `-oProxyCommand=` lacked. Per-folder VPN toggles broke this once and were removed. If they return, so does `vpn_approved` and nothing runs until someone on this machine has read it.
- All writes go through `config.rs` via `toml_edit` on a `DocumentMut`, never a re-serialized struct: hand-edited comments must survive (a test asserts it).
- Writes are temp file + `rename`, after `keep_a_copy` saves `<name>.bak`.
- Every writer has a `*_at(path, …)` twin the tests drive against a scratch file.
- Folder rename/delete are prefix rewrites of every jack's `folders` (`map_folders`). Deleting a folder keeps the device.

## Non-obvious

- **`$PATCHBAY_CONFIG`** overrides the config path. `scripts/dev.mjs` sets it to an absolute `dev/patchbay.toml` (`tauri dev` runs from `src-tauri/`) and puts `~/.cargo/bin` on PATH. `npm test` and `npm run check` go through it too. No env-var prefix in npm scripts: cmd.exe can't.
- Config lives in `%APPDATA%\patchbay\` on Windows, `$XDG_CONFIG_HOME` or `~/.config` elsewhere.
- **Releasing is a commit**: `npm run bump`, commit `release x.y.z`, push `main`. CI's part is in `.github/workflows/ci.yml`'s header.
- Release notes are the `CHANGELOG.md` section. Release body, `latest.json`, update pill and site all read it.
- **The update signing key is irreplaceable.** Private half `~/.tauri/patchbay.key` (CI: `TAURI_SIGNING_PRIVATE_KEY`), public half in `plugins.updater.pubkey` of every build. Lose it and no installed copy can update again. The endpoint is the release's `latest.json`. Without one `update_check` fails silently.

## Scope

- Sessions exec the system `ssh` and `sftp` with `patchbay.rs`'s argv on a real pty, so agent, `~/.ssh/config`, `known_hosts` and password prompts come free. **Never embed an ssh implementation.** "Open in Terminal" stays.
- Remote desktop is decoded in-window by IronRDP (pure Rust). VNC by vnc-rs. Royal TS's failure was bundling FreeRDP, a C graphics stack with its own CVE feed: **never link FreeRDP, no H.264 library** (the OS decodes, `h264.rs`). Handoff to a system client stays. In-window is the default.
- **patchbay stores and asks for no credentials.** Exception: an RDP/VNC password ticked Remember, in the OS keychain, never a file. Bitwarden's vault and sign-in are Bitwarden's own code from the installed app. patchbay never sees them.
- Watch memory per session: xterm scrollback is capped at 5000 lines. An RDP session holds a framebuffer plus the pipeline's surfaces and cache.
- Grouping is a `folders` **list** (`prod/eu/web` nests), so a jack sits in several branches. Never collapse it to one `group`: one home per host is what makes Royal TS's tree annoying. No tags, by decision. An old `tags` key is neither read nor written.
- **A name is unique per folder, a key per list.** `[jack.<key>]` is what jumps, links and commands use. A second `web` in another folder saves as `[jack.web-2]` with `name = "web"`, shown via `label`. `clash()` in `config.rs` checks on save and move.
