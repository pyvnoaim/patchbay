# patchbay

SSH connection manager: TOML files, exec `ssh`. A deliberately tiny answer to
Royal TS - no Electron, no sync service, no stored credentials.

**One file.** Your devices live in `patchbay.toml` and nowhere else. `folders` is a
list on each device, so grouping is a property of the device rather than a second
place to put one - and a folder can carry a note, which is the only thing that is
*about* a folder rather than in it.

One front end over one config format:

- `src-tauri/src/patchbay.rs` - config load, `[defaults]` inheritance, jump-chain walk, name resolve, and how a device is reached. All the logic worth testing lives here, with its tests at the bottom of the file.
- `src-tauri/src/import.rs` - a list you already have, in; jacks out. Two readers, one shape: an ssh config, and a Royal TS document. A `.rtsz` is XML despite the z, one flat list of objects joined by `ParentID`, so a folder path is a walk up the parents. Parses only; the window shows what it found and writes what you tick through `save_jack` like any other edit. Passwords are never read across, deleted objects are left in the trash, and a connection type patchbay can't open is named in the warnings rather than guessed at.
- **The list and the config can be two files.** `[settings].list` in the *own* config names a shared list, and devices, `[defaults]` and `[folder]` notes are read and written there; `[settings]`, `[colors]` and everything beside the config stay on this machine. `list_file()` in `commands/mod.rs` is the one place that resolves it, and it is where a missing shared list is an *error*: `load` reads NotFound as a first run and `read_doc` starts from the template, which on an unmounted share would paint an empty list and then write a one-jack file where the share mounts. Only `seed_list_at` writes at a path that isn't there, and never into a directory that isn't. Every command touching the list is async: a stat on a share that has gone away hangs, and a sync command hangs the main thread with it. Edits carry a `stamp` (a hash of the raw table as read) and `save_jack_at`, `delete_jack_at` and `set_note_at` refuse one that no longer matches, as does `save_defaults_at` over `[defaults]` (a missing table stamps empty, so an *addition* is caught too); the sheet takes the new stamp on refusal so the second save is a decision, not a loop. Under all of that, `read_doc` hands back the bytes it parsed and `write_doc` refuses to rename over a file that has moved since: every writer here replaces the whole document, so a colleague's save landing in between would otherwise go with it, devices the write never touched included. `list_stamp` hashes the file rather than reading its mtime and size - a sync client keeps the source machine's mtime, and a size doesn't move when `2222` becomes `2223`.
- **There is no licence check, and there is not going to be one.** patchbay is free under [PolyForm Perimeter](https://polyformproject.org/licenses/perimeter/1.0.1) at a company of any size; the only thing the licence forbids is selling a competing product built out of it, and that is not something a running copy can measure. `licence.rs`, the signed-blob format, the `licence` / `save_licence` commands and `npm run licence` were all deleted with the size clause they enforced. `writable_list()` in `commands/mod.rs` is the seam the wall stood in and stays as `list_file()` with a name, because every writer already goes through it. If a paid part ever arrives, this is the file to read in `git log` rather than the code to reinstate: an activation ping would be the only outbound request patchbay makes besides the update check, and that was the argument against it.
- `src-tauri/src/config.rs` - the only code that *writes* a config. Everything else reads. Also owns `[settings]`, `[colors]`, `[folder]` notes, and `fold_spaces_at`, the one-way door out of spaces: it writes the merged list before renaming anything, because the other order loses the devices outright if the write fails.
- `src-tauri/src/pty.rs` - in-app sessions: ssh on a real pty, streamed to xterm.js as `pty:<id>` events.
- `src-tauri/src/sftp.rs` - files, by handing them to `/usr/bin/sftp`. Stateless commands sharing one ssh session through `ControlMaster`, because a fresh handshake per directory listing makes browsing feel broken. `sftp` takes `-P` for the port where ssh takes `-p`, and its batch language is line-based - so a path with a newline or a quote in it is refused, the same way a `.rdp` refuses one. The writes (`edit`) are one batch line each behind a verb matched exactly; there is no recursive remove, so a folder with anything in it is refused rather than emptied. **"Edit here" is a download, the desktop opener and one `notify` watcher over the copies' folders** (`Edits` in `commands/files.rs`) - the far end knows nothing about a file being open, so a save is the only signal there is. The *folder* is watched, not the file: editors save by rename, and a watch on the file follows the old inode into the bin. The mtime is still compared, because one save is several events.
- `src-tauri/src/rdp.rs` - remote desktop by handoff: writes a `.rdp`, and forwards a local port over the jump chain when there is one. ssh's stderr is read on its own thread (a pipe nobody drains blocks ssh) and its last line is the error a failed tunnel shows; `list()` drops a tunnel whose ssh has since exited and hands the reason back, so the window can say so. Its `Tunnels` and `free_port` are what `vnc = <port>` uses too - VNC is a handoff and nothing else, `vnc://` to whatever viewer the machine has, so there is no `vnc_session.rs` and adding one needs the same argument `rdp_session.rs` had to win.
- `src-tauri/src/rdp_session.rs` - the other remote desktop: IronRDP decoded to a framebuffer and blitted onto a `<canvas>`, the way `pty.rs` streams a terminal. The only place patchbay speaks a protocol itself. **A resize is a round trip, not a CSS scale**: the pane's new size goes down the Display Control channel as one more `Input` on the session's queue (only that thread may touch the socket), the server answers with a Deactivate All, and `reactivate` re-runs the capabilities exchange on the same socket - rebuilding the fast path processor too, because the frame acknowledgements carry the old share id. The new size reaches the window as a tile header with no pixels behind it, in line with the tiles so nothing painted before the change is dropped. A server without the channel says nothing and the canvas letterboxes, which is also what the window shows for the round trip.
- `src-tauri/src/clipboard.rs` - the CLIPRDR backend behind `rdp_session.rs`. Text only, both directions lazy. The local side is polled (no desktop tells an unfocused process the clipboard changed), but `stamp()` asks the OS change counter first - NSPasteboard's `changeCount`, `GetClipboardSequenceNumber` - so a tick is one integer, and the text is only read once it moved. X11 has no counter, so there the text is the comparison.
- `src-tauri/src/webext.rs` - Bitwarden's own extension over the web tabs, through `WKWebExtension` (macOS 15.4+), off unless ticked. **Loaded from the installed Bitwarden for Mac, never bundled:** the Chrome build asks for offscreen documents and a side panel WebKit doesn't have, and its worker waits on them forever. patchbay answers as a browser in `shim` - a tab per web tab, one window, a delegate that draws the popup - and everything else takes WebKit's default, which denies. **Host access is the device urls in the list plus Bitwarden's own servers, never all hosts**; without the servers its fetches fail CORS and sign-in dies. Its web views say `Safari/` in the user agent because that is how Bitwarden works out where it is, and a bare WKWebView leaves it off. The context's `uniqueIdentifier` and `baseURL` are pinned to `BITWARDEN_ID`: left at WebKit's random default the extension's storage is memory-only and every launch is signed out, and changing the value later signs everyone out. WebKit state is main-thread-only and lives in a thread-local; `RUNNING` is the atomic that command threads read, because the thread-local is empty there.
- `src-tauri/capabilities/default.json` - grants `core:default`. Load-bearing; see Non-obvious.
- `src-tauri/Info.plist` - merged into the macOS bundle. Load-bearing: without it a plain-http device page is a silent white pane.
- Files kept *beside* the config, never in it, because each is this machine's answer rather than part of the list: `web_trusted` (checks waived), `rdp_known_hosts` (certs seen).
- `src-tauri/src/terminal.rs` - the other path: hands the ssh command to the *system* terminal. On Linux `$TERMINAL` is tried first, with the flags of the emulator it names if that is one we know and `-e` otherwise.
- `src-tauri/src/main.rs` - Tauri setup: plugins, the macOS menu, the command list. `src-tauri/src/commands/` - the `#[tauri::command]` surface, one file per area (`jacks`, `settings`, `sessions`, `web`, `remote`, `files`, `app`). Thin; logic belongs in `patchbay.rs`.
- `ui/` - no framework, no bundler. **Classic scripts sharing one global scope, so the order in `index.html` matters**: `core.js` (state + helpers) → `browse.js` (tree, list, detail, palette) → `sessions.js` (terminal tabs) → `edit.js` (menu, sheets, config writes) → `boot.js` (wires the keyboard and starts up; the only file that *runs* rather than declares). `theme.js` is the exception on both counts: it runs, and it runs from `<head>` before the body paints, because the theme is an attribute and the first frame needs it. Anything reaching across files must do it inside a function body, never at top level.
- `ui/gen/` - generated by `npm run icons` / `npm run brands`. `ui/vendor/` - third-party (xterm), by `npm run vendor`. Committed, but never edit either by hand.
- `dev/patchbay.example.toml` - the sample config, tracked. `dev.mjs` copies it to `dev/patchbay.toml` on first run; that copy is gitignored, because it fills up with your own machines and a real address belongs in a public repo about as much as a password does. Everything else `dev/` accumulates (`web_trusted`, `rdp_known_hosts`) is ignored for the same reason.
- `CHANGELOG.md` - the release notes, and the only place they are written. `npm run bump` stamps `## Unreleased` with the version and date, `scripts/notes.mjs` hands that section to the release workflow as the GitHub release body, and from there the *app* reads it (the notes ride along with the update check) and the *site* fetches it back from the releases API. Written for someone about to install, not for someone reading the diff.
- `scripts/` - one-job node scripts, run via npm. Never imported by the app; `dev.mjs` is the single entry point for running anything locally. `draw.mjs` is the exception to "one job": a PNG writer and the SDF helpers that `icon.mjs` and `dmg.mjs` both draw with, so neither needs an image dependency.

## Commands

```sh
npm run dev      # the app window, against dev/patchbay.toml
npm test         # cargo test, through dev.mjs so PATH and the config path are right
npm run check    # prettier --check, cargo fmt --check, cargo clippy -D warnings; what CI runs
npm run fmt      # prettier --write and cargo fmt
npm run build    # patchbay.app / .exe / .deb; on macOS opens the .dmg it made
npm run icon     # regenerate the app icon from scripts/icon.mjs
npm run dmg      # regenerate the .dmg window background from scripts/dmg.mjs (macOS: needs tiffutil)
npm run bump [0.1.0] # the version (next patch if none), in the four files that carry it
npm run icons    # regenerate ui/gen/icons.js after editing USED in scripts/icons.mjs
```

## "push"

When I say **push**, that is not just `git push`. It is the whole procedure, and it is
the *only* time anything is committed:

1. **Take everything open.** Uncommitted changes, staged and unstaged, new files, and anything already committed but unpushed. The review covers the lot, not just the last thing worked on.
2. **Review it all**, in these four passes:
   - **Bugs and correctness** - especially in `patchbay.rs`, where a wrong argv is a connection that fails somewhere else entirely.
   - **Security** - problems *and* improvements. Anything reaching a shell, the trust store, a `style` attribute, the desktop opener, or a file someone else wrote.
   - **Performance** - problems *and* improvements. Something that got slower, an extra round trip on a hot path, work done before the user sees anything.
   - **Simplification** - what can be deleted, reused, or replaced by something already here.
3. **Run `npm run check` and `npm test`** - the whole crate, not one module's `#[cfg(test)]`.
4. **Clean → commit and push.** Sensible commit messages, split into separate commits when the changes are unrelated.
5. **Then release.** If `## Unreleased` in `CHANGELOG.md` has entries, `npm run bump` (next patch), commit `release x.y.z` as the last commit and push `main`; there is no tag to push. CI builds all four and publishes, and every installed copy offers the update on its next check. A push with nothing someone would notice releases nothing: an update that changes nothing still costs everyone a restart.
6. **Not clean → stop and tell me what you found.** Don't push and don't fix it silently; a behaviour change is my call. Trivial nits (a typo, dead code, a stale comment) you can just fix, mention, and carry on.

Findings first, one line each. Don't push a "probably fine".

**Never commit or push unless I've asked for it in that message.** Not after a
cleanup, not because the work looks finished, not because the tree is clean, and not
because a change is small. Finishing a task is not permission to record it - leave
the work in the tree and say what's ready. Both `git commit` and `git push` only ever
run when I say **push**.

## Conventions

Read a neighbouring file before adding one; match what's there. The rules that
aren't obvious from reading:

**Everywhere**

- Comments explain *why*, never *what*. If a line needs a comment to say what it does, rename something instead. The exceptions worth writing: a non-obvious ordering constraint, a platform quirk, a deliberate shortcut.
- Mark deliberate simplifications `ponytail:` with the ceiling and the upgrade path - `// ponytail: one thread per jack, bounded pool if someone brings a thousand`.
- Prefer deleting to adding. No interface with one implementation, no config for a value that never changes, no scaffolding for later.
- Errors are lowercase sentence fragments naming the thing that failed, and quote the user's input: `no jack named "web"`, `jump loop through "loop2"`. They reach the user through the window, so phrase them for someone reading a sheet, not a stack trace.

**Rust (`src-tauri/src/`)**

- `Result<T, String>` at the command boundary; the string is shown to the user, so it follows the error style above.
- Platform code is `#[cfg(target_os = ...)]` in `terminal.rs`, never an `if` on a runtime flag.
- Pure logic goes in `patchbay.rs` and gets a test there. `commands/` stays the thin surface over it; a helper with a test of its own may sit beside the command that uses it.
- `npm run check` must pass: rustfmt, clippy with warnings denied, prettier. Run `npm run fmt` rather than formatting by hand.

**Frontend (`ui/`)**

- No bundler, so no `import` - scripts are classic, loaded in order by `index.html`. `withGlobalTauri` is on; call `window.__TAURI__.core.invoke`.
- CSP is `script-src 'self'`: no inline `<script>`, no CDN. Inline `<style>` is allowed but put styles in `app.css` anyway.
- Everything interpolated into `innerHTML` goes through `esc()`. No exceptions - a jack name comes from a file a colleague may have written.
- Colours only from the `:root` custom properties, and every one needs its light-mode value in the `prefers-color-scheme` block. Never hardcode a hex outside `:root`.
- Icons are Lucide via `icon("name")`. Add the name to `USED` in `scripts/icons.mjs` and run `npm run icons` - don't paste SVG into `app.js`, and don't add `lucide-react` (there is no React here, and it wraps the same artwork).
- A jack's `os = "..."` renders through `osIcon()`: a simple-icons brand mark if one exists, else a Lucide shape from `BRAND_FALLBACKS`, else `server`. Both maps live in `scripts/brands.mjs`; run `npm run brands`. Matching is loose on purpose so `"Ubuntu 22.04"` and `"ubuntu"` land on the same glyph. Brand marks are *filled* paths, Lucide ones are *stroked* - `.i.brand` clears the stroke.
- Don't fetch favicons from devices to use as icons. It needs an HTTP client and TLS in the app, nearly every appliance ships a self-signed cert, half of them sit behind a bastion where the app can't reach them anyway, and it turns opening the window into outbound requests to every host. The curated set covers the real cases.
- **A device is reached one way.** The sheet's row is a radio, and saving clears the other two - so a device edited there keeps only what it was set to, including one written back when the row allowed several. `primary` is still *read* (old configs, and hand-edited ones can still set two), and it is still resolved so an option pointing at something the device no longer has falls back rather than doing nothing - but the window no longer writes it, because with one field set the answer is derivable. That resolution is `primary()` in `patchbay.rs`, with tests: `primary` only picks the default action, so a device with a `url` *and* ssh still has both.
- **Anything spawned detached must be killed on exit.** A pty session dies when its master fd closes, but `ssh -N -L` does not - `RunEvent::Exit` calls `close_all()`, or tunnels outlive the window holding their ports.
- `.rdp` is line-based, so a newline in a host or username injects directives - `alternate shell:s:` runs a program. Control characters are rejected before anything is written.
- Brand colours are unusable raw - nine of the 29 fail contrast on one theme. `readable()` nudges lightness until a colour clears 3:1 against the current surface; never paint a brand hex directly.
- **Anything that goes wrong outside a form goes through `alertish()`, which flashes a pill.** It used to write into the detail pane's command box, which isn't there when a folder is selected and is gone entirely under 720px - so a failed delete could say nothing at all. `#notices` holds two pills, not one: an error arriving mid-download must not take the update's own Restart button off the screen.
- **A new full-screen overlay has to be registered in three places besides `index.html`, and all three failures are quiet:**
  - `OVERLAYS()` in `core.js` - feeds `modalOpen()` and the observer that shrinks a web tab away. Left out, the webview paints straight over your overlay. (The context menu was missed first time round: the sidebar stays live while a session tab is open.)
  - the `#sheetwrap, #askwrap, #importwrap` rule in `app.css` - left out, it has no `position: fixed`, sits in normal flow at the end of `<body>`, and stretches the layout behind it.
  - the modal guard in `boot.js` - left out, Escape doesn't close it and every global chord still fires; ⌘N opens the jack sheet on top of it.
- **The find bar is the second thing deliberately *not* in `OVERLAYS()`**, for the update pill's reason: `modalOpen()` would hand the window's chords to a find box and shrink a web tab away every time you looked through a shell. It lives inside `#terms`, so the pane hides it, and it takes Escape on its own input. Its chord is ⌘F, and it was the first to be **Ctrl+Shift+F elsewhere** - plain Ctrl+F is readline's forward-char, and a session owns the keyboard. Every chord is that now: `chorded(e)` is ⌘ on macOS and Ctrl+Shift off it, and `chordKey(e)` reads punctuation off `e.code` because Shift turns `[` into `{`.
- **Delete hands back what it removed.** `delete_jack` returns a `Removed` - the table as a one-table TOML document plus the position it had - and the pill's Undo sends it to `restore_jack`, which makes room at that position (`make_room`: positions are renumbered by every parse, so the old slot is taken by whatever followed). The comments above a deleted jack are rehomed to the next table and *stay there* through an undo: stale is visible, duplicated is not. Held in the window for the life of the pill, never in the config - a wrong "yes" is the case, not a history.
- **A drag is pointer events, not HTML5 drag and drop.** The window's file-drop handler (uploads into a files tab) takes the native drag over on some platforms, and a web tab is an OS view above the page that would swallow a `dragend`. `pointerdown` on a row, a six-pixel threshold, `elementFromPoint` for the folder under the pointer, and the click that ends a drag is stopped in the capture phase so it doesn't reselect. A drop writes only `folders`, through `set_folders`, so a hand-written key on the device survives; `moveJacks` is the one place the semantics live - a *move* out of the folder you are looking at, an *add* from All devices.
- **Marks are names, the selection is an index.** ⌘-click and shift-click pick rows out into `marked`; the list is refiltered under them by every render, so an index would follow whatever moved into that row. The first ⌘-click also marks the selected row, the way shift-click always has: otherwise three rows are lit, the dock says two, and the blue one is the one Delete spares. `markedHere()` is what a bulk action applies to - a mark on a device you have since filtered out of view is not part of what was asked for. **Broadcast** is one of those actions: `openBroadcast()` in `sessions.js` opens every ssh mark as its own pane inside a shared tab, and their `s.bcast.on` fans one `onData` out to every live sibling. Non-ssh marks are named in a pill and skipped - a webview or an RDP canvas mixed into the grid layout is a much bigger change than the feature is.
- **A selected row is `{ path }`, not a bare string.** Everything keyed by a folder - `expanded`, `pending` - goes through `gkey()`, so there is one shape for "which folder" and one place to change it.
- **A pane being typed into is never redrawn.** `renderDetail` returns early when the focus is inside it: the probe sweep re-renders every thirty seconds, and `innerHTML` takes a half-written folder note with it.
- One render path: mutate state, call `render()`. No targeted DOM patching - the lists are tens of rows, not thousands.
- **Only the palette ranks by recency.** Every open (`connect`, web, RDP, VNC, files) calls `used(name)`, which keeps the last forty names in `localStorage` - a habit, not part of the list, so it never touches the config. The list is in file order unless the header's Sort says A–Z, type or device (`sortJacks` in `browse.js`), and never by recency: rows moving under someone reading them is a worse list, not a smarter one. The theme, the sort, the tabs to reopen, and the sidebar's open folders (`keepExpanded()`, read back once per launch, so a folder folded stays folded and a fresh machine starts with all of them closed) are there for the same reason. Those five are everything `localStorage` holds, and every one of them is a habit this machine has rather than something the list says.
- The default `contextmenu` is suppressed app-wide (it's the webview's Reload/Inspect menu). Right-click is ours; new actions go in the `contextmenu` handler as well as a visible button, since a menu alone isn't discoverable.
- Shortcut labels come from `chord("k")`, never a hardcoded `⌘` - it reads `Ctrl+Shift+K` off macOS. Matching goes through `chorded(e)` and `chordKey(e)` for the same reason; never `e.metaKey || e.ctrlKey`, which made Ctrl+W in a shell a close prompt.
- **A live session owns the keyboard.** The global `keydown` handler returns early when `activeId !== null`; every keystroke belongs to ssh. Only window-level chords (⌘K, ⌘N, ⌘W, ⌘[/]) may be intercepted, and each one you add is a key someone can no longer send to their remote shell. The terminal's own keys are `termKey()` in `sessions.js`: on macOS ⌘ never reached the shell, so ⌘A, ⌘+ and ⌘← cost nothing, and ⌥← is *remapped* to readline's `ESC b` rather than taken. **On macOS a ⌘ key the menu bar has (⌘A, ⌘C, ⌘V, ⌘Z) never reaches a `keydown`**: `unstable` makes the page a child webview, and wry sends a child's ⌘-keys to the menu first. Catch the menu's effect instead, the way ⌘A is a `selectstart` on xterm's textarea. Everything that sends bytes stops at the shell's screen: in vim, ⌘←'s `^A` would increment a number.
- xterm needs a laid-out element to size itself, so `fit()` after the pane is visible, not before.
- **Core commands need a capability; ours don't.** Anything declared with `#[tauri::command]` and listed in `generate_handler!` works with no permission at all, but Tauri's own APIs - `listen`, `emit`, clipboard, path - are denied unless `src-tauri/capabilities/*.json` grants them. Deleting that file doesn't break `invoke("jacks")`, it just makes sessions open and sit there mute, which reads like a UI bug and isn't. If a `window.__TAURI__` call rejects for no visible reason, check the capability first.
- **"Ours don't" means ours don't *from the local origin*.** A device's web UI is a child webview, and that page is an `Origin::Remote`, which matches no `ExecutionContext` in `capabilities/` - so it reaches none of our commands. That is the only thing between a Synology's login page and `delete_jack`. **Never add `remote` to a capability**, however reasonable the reason looks.
- The web UI is a **tab**, like ssh and RDP - a child webview via `Window::add_child`, which is why `tauri` carries the `unstable` feature. Never an iframe: DSM, OPNsense and Proxmox all send `X-Frame-Options`, so an iframe would work only for the devices nobody points a `url` at.
- **A child webview is an OS view above the page.** It obeys no CSS of ours - not `hidden`, not z-index, not a sheet - so it is either sized exactly over its host div or sized to nothing. Tooltips check `webViewRect()` and flip, or don't draw.
- A webview has no certificate interstitial: an untrusted cert paints an empty pane and offers nothing. `web_check` runs *alongside* the tab, not in front of it, and `on_navigation` follows redirects a http client can't see - a Synology's http port is three lines of JavaScript pointing at its https one.
- **Cleartext is LAN-only, enforced in `is_private_host`.** `Info.plist` turns ATS off for web content because Apple offers nothing narrower that covers a bare `192.168.x.x`; the narrowing is ours, so a public `http://` url goes to the browser instead.
- A web tab's label carries the **url**, not just the jack name. Keyed on the name alone, the first view opened for a device was reused by every later click - the check never re-ran and the page never changed.

**The site (`site/`)**

- **Three rules decide how lines break, and everything obeys one of them.** Headings and `.lede` get `text-wrap: balance` - they are one or two lines read in a single glance, so even lines beat a full first one. Every other `p` gets `pretty`, which only rescues the last line, the one that strands a word. And a term that reads as one thing never splits: `.tt` for the inline mono ones, `.keep` for a pair of ordinary words like "ssh agent". Reach for `.keep` before rewriting a sentence to dodge a bad break.
- **Only mock data on the page.** Hosts are `10.0.0.x`, the customer is `Acme GmbH`, the document is `acme.rtsz`. A real address or a real customer's name on a public page is somebody else's data, and it has happened twice.
- **Every animation is added by `script.js`, never assumed by the markup.** The reveal sets `data-rise`, the patch panel draws its own leads and sockets. A blocked script costs the page its motion and nothing else - it never leaves a blank band or an invisible section.
- Anything positioned against another element's box is measured, not guessed: the patch panel's sockets are drawn in the same SVG as the leads, because as CSS they were positioned against the line and the leads against the panel, and the two disagreed by a few pixels no matter how they were written.

**Validating what reaches the outside**

- Anything that reaches a `style` attribute or the desktop opener is validated on the way in *and* on the way out: `[colors]` must be `#rrggbb`, a jack's `url` must be http(s). Both are checked in Rust on save and again in JS before use.

**Writing the config**

- **Nothing in a config file is executed as written.** The most one can produce is an `ssh` argv, and `dest()` in `patchbay.rs` is where that is enforced: a `host` or a raw `jump` beginning with `-` is an ssh *option*, and `task_argv` appends `traceroute <host>` after the chain, which is the operand a smuggled `-oProxyCommand=` was otherwise missing. Per-folder VPN toggles broke it once and were removed; if they come back, so does `vpn_approved` and the rule that nothing runs until someone on this machine has read it.
- All writes go through `config.rs`, via `toml_edit` on a parsed `DocumentMut` - never re-serialize the struct. People hand-edit this file and their comments must survive; there's a test asserting exactly that.
- Writes land as temp file + `rename` so a crash can't truncate someone's hosts, and `write_doc` first keeps the file it is about to replace as `<name>.bak` beside it (`keep_a_copy`). One generation, quiet on failure: the window's Undo is a pill that only covers a delete, so the copy is the only way back from a folder rename that rewrote every device in it.
- Every write function has a `*_at(path, …)` twin that the tests drive against a scratch file. Add the twin when you add a writer, or it can't be tested without touching a real config.
- Folder rename/delete are prefix rewrites of the `folders` list on every jack (`map_folders`) - deleting a folder drops the entry and keeps the device.

## Non-obvious

- **`$PATCHBAY_CONFIG`** overrides the config path - that's how you run the app without touching `~/.config`. Everything local goes through `scripts/dev.mjs`, which sets it to an absolute `dev/patchbay.toml` (absolute because `tauri dev` runs the binary with `src-tauri/` as its cwd) and puts `~/.cargo/bin` on PATH so a terminal opened before rustup still works. That's why `npm test` and `npm run check` shell through it too. Don't reintroduce an env-var prefix in an npm script - it doesn't work in cmd.exe.
- Config lives at `%APPDATA%\patchbay\` on Windows, `$XDG_CONFIG_HOME` or `~/.config` elsewhere.
- **Releasing is a commit.** `npm run bump 0.1.0`, commit `release 0.1.0`, push `main`. `.github/workflows/ci.yml` is the one workflow: `test` runs on every push, and when the pushed head is a `release x.y.z` commit it also builds macOS arm64 and x64, Windows and Linux, and puts them in a **draft** release with a `latest.json`, an `install` job installs the draft's own files on each OS (the `.dmg` has to pass `codesign --strict`, or a download is "damaged" with no Open Anyway - that is what `signingIdentity: "-"` is for), and a last `publish` job lets it out - which is what creates the tag - once every one and `test` passed and `latest.json` has a signed entry per platform. Publishing is the moment every installed copy starts offering the update, so a failed leg leaves the draft unpublished rather than offering a release one platform is missing from - a draft is not `releases/latest`, so nothing is offered until it publishes. The `version` job compares the commit message against `tauri.conf.json`, refuses a version already tagged, and makes the draft the four legs upload into. Builds run on `main` and never on a tag because a cache saved under a tag is read by that tag alone, which made every release a cold build.
- **The release notes are the changelog section, and nothing retypes them.** `CHANGELOG.md` → release body → `latest.json` notes → the pill's "What's new", and separately the site's `#changelog`, which fetches the releases API on scroll. The notes are *not* covered by the update signature (that covers the archive), so the window escapes them like any other outside text and Rust cuts them at 4000 **characters** - `String::truncate` is bytes and panics mid-codepoint.
- **The update signing key lives outside the repo and can never be replaced.** Private half at `~/.tauri/patchbay.key` (CI signs with `TAURI_SIGNING_PRIVATE_KEY`), public half baked into every build via `plugins.updater.pubkey` - so losing it doesn't mean regenerating, it means every copy already installed stops being updatable. The endpoint is the `latest.json` on the GitHub release, which only exists once a release publishes one; until then `update_check` just fails and says nothing.
- The update check is ours, not the plugin's JS API - `update_check` / `update_install` / `update_restart` in `commands/app.rs`, so it needs no capability and the window only ever sees a version string. `boot.js` runs it once after the first paint and the offer is `#uptoast`, a pill at the foot of the window rather than a sheet: an update is news, not a question. It is the one floating thing that is *not* in `OVERLAYS()` - putting it there would make `modalOpen()` true and give a one-button pill the keyboard. `update_install` emits `update:progress` a whole percent at a time and the pill's own button is the bar, filled through `--p`; no percent yet means the pulse, which is also the answer for a server that sends no content-length. **The install never restarts you** - the new bundle is swapped in, the same button becomes `Restart`, and `update_restart` is the second click. A restart takes every live session with it, so it is never something a finished download does to you. Two ways to ask for one yourself - "Check for Updates…" in the macOS menu (which only emits `menu:check-update`; what a check *looks* like stays in the window) and the button in Settings ▸ Updates, where the answer also has to appear beside the button because the pill is behind that sheet. A check someone asked for answers either way; the launch one, gated on `check_updates` in `[settings]`, stays quiet unless there is something to install.
- **The disk image ejects itself**, from `eject_install_image()` at startup: nothing in macOS ejects it when the drag finishes, and the app that came out of it is the only thing that knows it is there. Never when running *from* the image, which is someone trying it before installing.
- **One config file.** Spaces were extra files from when a shared list had to be one of its own; folders do that job now. `fold_spaces_at` runs at startup and folds anything still in `spaces/` into the list, each device keeping the space's name as its outermost folder and the space's `[defaults]` written onto it. It writes the merged list *before* renaming what it read, because the other order loses the devices if the write fails.
- Jump chains resolve by walking `jump` until it hits a non-jack name (passed through raw) or runs out. The cycle guard is load-bearing; don't drop it.
- **A `forward` is `-L` unless it says otherwise** - `-R <spec>` and `-D <spec>` are written as ssh's own flag, and `forward_arg` matches those three exactly and refuses anything else starting with `-`. That check is the whole safety story: the value becomes argv, and `-L` used to be safe for free by never being anything but an operand. Everything that *strips* forwards goes through `FORWARD_FLAGS`, or an sftp run keeps the `-D` a session already bound. A `-R` has no local port, so `forward_local` gives `None` and `Tunnels::open` judges that tunnel by ssh still being alive.
- **`-J` order is reversed relative to the walk.** Walking `jump` goes *outward* from the target, but `ssh -J a,b` dials `a` first. `db → web → bastion` must emit `-J bastion,web`. There's a test pinning it - that bug is invisible until a chain is three deep.
- **`patchbay://` links come through `tauri-plugin-deep-link`, from Rust only.** The scheme is declared once in `tauri.conf.json` and the bundler registers it per platform; on Windows and Linux a link is the argv of a *second* process, which `tauri-plugin-single-instance` (with its `deep-link` feature) hands to the running one. No capability is needed because the window never calls the plugin - `deliver_link` resolves the name and emits `open:link`. macOS has no runtime registration, so testing a link there means a built bundle, never `tauri dev`.
- The status dot probes the chain's *entry point* (the outermost bastion), not the target. Anything past the first hop is only reachable through ssh, so there's nothing to TCP-probe.

## Scope

We shell out to `/usr/bin/ssh` on purpose - the agent, `~/.ssh/config` and
`known_hosts` come free. Do not add an ssh2 client or a credential store.

There **is** a GUI now (Tauri, not Electron), and it **does** host sessions in-app:
`pty.rs` spawns `/usr/bin/ssh` on a real pty and streams it to xterm.js. That is a
deliberate reversal of the original "launcher, not a client" rule. What did *not*
change is the part that matters: we still exec the system `ssh` with the argv
`patchbay.rs` builds, so the agent, `~/.ssh/config` and `known_hosts` still do the work,
and a real tty means password and host-key prompts behave. "Open in Terminal" is
still there on the context menu.

Remote desktop went the same way, and further: `rdp_session.rs` decodes RDP itself
via IronRDP and paints it on a canvas. Read the objection precisely before calling
that a reversal - the Royal TS failure mode was bundling *FreeRDP*, a C graphics
stack with its own toolkit, window handling and CVE feed. IronRDP is pure Rust, no C
dependency, and the pixels land in the webview we already ship. So the rule that
stands is: **never embed an ssh protocol implementation, never link FreeRDP.**
`rdp.rs` keeps the handoff to mstsc / Windows App / xfreerdp, and it stays - an
in-window session is the default, not the only way. Watch memory per session; xterm
scrollback is capped at 5000 lines on purpose, and an RDP session holds a full
framebuffer.

The Bitwarden host reads like the credential store this app refuses, and isn't: the
vault, the master password and the sign-in are Bitwarden's own code in Bitwarden's own
web views, and patchbay never holds or sees any of it. What stands is **patchbay stores
no credentials and asks for none.** Its traffic to Bitwarden's servers is Bitwarden's,
and only while ticked; the extension comes from the Bitwarden for Mac already
installed, so patchbay ships nobody else's code.

Grouping is a `folders` list on each jack - a string with slashes (`prod/eu/web`)
nests in the sidebar. It's a **list**, and that's the load-bearing part: a jack sits
in several branches at once. Don't collapse it to a single `group` field - one home
per host is what makes Royal TS's tree annoying to navigate.

There is no separate tag concept, and adding one was considered and rejected: a
folder was the only thing anyone used it for, and two overlapping ways to group the
same hosts is the confusion this app exists to avoid. `tags` was once read as an alias
for `folders`; it is neither read nor written now, and an old file with one just has
that device unfiled.
