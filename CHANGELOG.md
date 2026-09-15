# Changelog

Written for someone about to install, not for someone reading the diff: what
changed for them, in plain sentences. Newest first.

This file is the source. `npm run bump` stamps `## Unreleased` with the version
and the date, the release workflow lifts that section into the GitHub release,
and from there the app shows it before it installs anything and the site reads
it back. Nothing is retyped anywhere.

## Unreleased

- Remote desktop: opening one no longer renegotiates the screen a moment after it
  appears, which is what made the first few seconds look like it was calibrating.

## 0.1.2 - 2026-09-15

- The command palette puts folders first, with a line between them and the devices.
- Remote desktop: right-clicking a session no longer opens the webview's own Reload
  menu over it.
## 0.1.1 - 2026-09-12

- Windows and Linux: the sidebar and the panels are no longer the same near-black.
  Without macOS's vibrancy behind them the two surfaces were within two shades of
  each other, so the whole window read as one flat slab.
- Windows 11: the title bar takes the app's own colour instead of the system's, so the
  window is one piece rather than two.

## 0.1.0 - 2026-09-11

- ⌘K finds folders too: pick one and the list opens on it.
- A device clicked in ⌘K opens the way Enter opens it. A click always started ssh.
- Sidebar folders sort A–Z. Ones with subfolders no longer go first.
- Folders can be marked with ⌘-click or shift-click too, and removed together.
- ⌘A marks every device in the list you are looking at, all of them or one folder's.
- A remote desktop is sharp on a Retina screen, at the size Windows would draw it there.
- A remote desktop fills its tab, without a black frame around it.
- FRITZ!Box is a device kind, with AVM's mark.
- The device list sorts A–Z, by type (web, ssh, remote desktop) or by device (Windows,
  Raspberry Pi…). Pick it from Sort at the top; the file keeps its own order.
- Terminal keys work like a Mac terminal: ⌘A selects all, ⌘←/⌘→ jump to the line's ends,
  ⌥←/⌥→ by word, ⌘⌫ deletes to the start, ⌘+/⌘-/⌘0 size the text.
- Off macOS, Ctrl+Shift+C and Ctrl+Shift+V copy and paste in a terminal.
- Find in a terminal finds things. ⌘F opened the box and then highlighted nothing.
- Bitwarden stays signed in when patchbay restarts. Unlocking still follows Bitwarden's
  own timeout.
- Bitwarden fills device logins. Tick it in Settings ▸ Bitwarden and each web tab gets a
  key: click to fill, right-click to open Bitwarden. Needs Bitwarden for Mac installed.
- Settings no longer opens behind a device's web page.
- Settings ▸ Team has a file picker for the shared list, and a lot less text.
- A button works on the first press. Coming back to the window redrew the pane under
  your click, so Ping, Files and the rest needed pressing twice.
- Trusting a device's certificate sticks. The page used to load and then be covered by
  the same "trust it" panel again, once for every address the page redirected through.
- Closing a session no longer asks first. Only closing several at once - a broadcast
  group, or quitting with live tabs - is still worth a question.
- The sidebar opens the way you left it. Folders you folded stay folded, and a first
  run starts with all of them closed instead of every one unfolded.
- An import from Royal TS keeps the way in each connection had: an RDP or web entry
  opens as a desktop or a web tab, not as ssh.
- A remote desktop follows the window: resize it and the desktop changes resolution to
  match, so there are no black bars and nothing to reconnect for. A host too old to be
  asked has its picture scaled into the pane instead.
- Keys no longer stick in a remote desktop. After a ⌘ chord or a switch to another app,
  typing went to Windows as shortcuts; everything held is released now.
- Free, all of it: every protocol, every tab and the shared list, at a company of any
  size. PolyForm Perimeter is the licence, and nothing in the app asks for a key.
- Every write keeps the file it replaced as `patchbay.toml.bak`, beside your config. The
  Undo pill only ever covered a delete, and only for as long as it was on screen; a
  folder rename touches every device in it.
- A shared list refuses more than it used to: deleting a device a colleague has changed
  since, or saving `[defaults]` over theirs, is refused the way an edit already was, and
  a whole file landing under a save is never replaced by it. A change on the share also
  shows up now when it leaves the file the same size.
- In the light theme the Delete button's gradient no longer runs into the dark theme's
  red halfway down.
- Settings ▸ Team points the list at a file on a share or a synced folder, so a team
  works from one list. Your settings stay in your own file, a device changed by someone
  else since you opened it is refused once rather than overwritten, and a share that is
  away keeps the list on screen.
- A remote desktop opens at the size of the pane it is in, not at 1280×1024 scaled to
  fit, so text is sharp and there are no black bars.
- A config file patchbay creates starts with every key as a comment, so opening it by
  hand on a first run shows what goes in it rather than a blank page.
- On Linux, "Open in system client" for a remote desktop tries xfreerdp before asking
  the desktop what opens a `.rdp`, which without Remmina was a text editor.
- On Windows and Linux the window's shortcuts are Ctrl+Shift+K, Ctrl+Shift+W and so on,
  not plain Ctrl: in a shell Ctrl+W deletes a word and Ctrl+[ is Escape, and both used
  to reach the window as well. Ctrl+W in a macOS shell no longer asks to close it either.
- Terminals and paths on Windows use Cascadia Mono or Consolas, and DejaVu Sans Mono on
  Linux, rather than whatever "monospace" falls back to.
- Deleting a device offers Undo for a few seconds. It goes back where it was, with
  everything it had.
- Drag a device onto a folder in the sidebar to file it there. Or right-click ▸ Move to
  folder…, which also works on several marked devices.
- A Shortcuts page in Settings lists every key. `?` opens it; so does the Help menu on
  macOS.
- On Linux, "Open in Terminal" honours `$TERMINAL` before trying the usual suspects.
- A forwarded port that fails after ssh has connected is reported with ssh's own words,
  and a tunnel that dies later is named the next time the list refreshes rather than
  sitting there looking open.
- "Edit here" hears about a save from the OS instead of checking the copy every two
  seconds, so the upload starts the moment you save.
- The reachability sweep dials each bastion once, however many devices sit behind it,
  and never more than thirty-two hosts at a time.
- The remote desktop clipboard checks the OS change counter, not the clipboard's
  contents, so a large copy no longer costs anything until it is pasted.
- The disk image's background is sharp on a retina screen.
- `tags` in a config file is no longer read as a synonym for `folders`. Nothing patchbay
  ever wrote used it.
- Clicking the empty space under the list clears the marks; the selection stays.
- Closing a tab, a broadcast group or the window with a live session in it asks
  first; a second ⌘W is the yes. A page, a file listing or a finished session closes
  without a question.
- A device sheet with something typed into it asks before Escape or a click outside
  throws it away.
- Duplicate is a button on the device's pane now, not just a right-click item.
- Left and Right arrows walk the folder tree: fold, unfold, step up or down.
- **Quick connect.** Type `user@host` or `host:2222` into the search and press Enter:
  a session, no record. Your `~/.ssh/config` still applies; nothing else does.
- The tabs you had open come back on the next launch: shells, web pages and file
  listings, in the same order. A remote desktop asks for a password, so it doesn't.
- A shell's tab carries its device's mark, so six shells on six boxes tell apart.
- The search palette puts the devices you opened most recently first. The list itself
  keeps the order you filed it in.
- **Import a Royal TS document.** Point patchbay at a `.rtsz` and it reads the lot:
  folders become folders, RDP and ssh connections and web UIs become devices with the
  way in they already had. Tick what you want; nothing else is written. Passwords stay
  in Royal TS, because patchbay keeps none.
- Importing is its own page in Settings now, with both sources on it, and the list it
  finds is grouped by folder with a filter - a document with a hundred devices in it is
  not a list you scroll.
- **One list, not several.** Spaces are gone: folders do the grouping, and anything you
  had in `spaces/` is folded into your list on the next launch, keeping its own name as
  the outermost folder. The files it read are kept as `.toml.merged`.
- A folder can carry a note - what somebody arriving at that customer needs to know -
  shown on the folder's own page.
- A device name can contain a dot. `dc1.acme.local` is a name people actually use.
- A setting to log a session's terminal output to a file under `logs/`, beside the
  config. Off by default.
- **Broadcast to marked devices.** Pick out several boxes, right-click, hit Broadcast:
  every device opens as its own pane in one tab, and one keystroke reaches all of them.
  Click a pane to focus it and one keystroke goes there instead; the tab's broadcast
  toggle turns fan-out on and off without closing anything. Marks a NAS's web UI along
  with the ssh boxes? The web ones are left out and named.
- A bulk-action bar appears when you mark two or more devices: broadcast to them or
  delete them without hunting for the right-click menu.
- A device whose address begins with `-` is refused rather than handed to `ssh`, which
  would have read it as an option. Nothing in a config file is executed as written, and
  now that holds for a ping or traceroute through a jump too.
- Tailscale addresses count as your own network, so a `100.x` device's web UI opens in
  a tab like every other one instead of being sent to the browser.
- A `patchbay://` link matches a device name exactly, and can carry a name with a dot
  in it. `patchbay://db` no longer opens `db-prod`.
- patchbay can write your devices into `~/.ssh/patchbay.conf` and have `~/.ssh/config`
  include it, so `ssh web-01` in any terminal goes where Connect goes. So do `scp`,
  `rsync` and anything else that reads that file, jump chains included. Off until
  you turn it on in Settings ▸ Devices, and a name your own config already defines
  is left alone.
- A previous install's `~/.ssh/patchbay.conf` and its `Include` line are found on
  launch and offered a one-click cleanup, for whenever the setting above is off.
- A **Map** view beside the list: devices grouped by the route to them, so a bastion
  and everything behind it sit together however you filed them. A bastion that isn't
  answering says so once, instead of every device behind it looking broken on its own.
- `patchbay://web-01` in a runbook or an alert opens that device, on every desktop.
  A second copy launched by a link hands it to the one already running.
- A device still being checked has a dot that says so, rather than looking the same
  as one nothing is checking.
- The file browser writes as well as reads: rename, delete, and a new folder to
  upload into, all on the right-click menu.
- **Edit here** on a remote file opens it in whatever this machine opens it with,
  and puts it back every time you save.
- A forward can be a remote one or a SOCKS proxy now, not just a local port:
  write `-R 9000:localhost:9000` or `-D 1080` in the same field. Importing an ssh
  config brings `RemoteForward` and `DynamicForward` across too.
- Find in a session: ⌘F, or Ctrl+Shift+F, searches that terminal's scrollback.
- ⌘-click and shift-click pick out several devices at once, to act on them in one go.
- A device's forwards can be held open on their own, without a session, from the
  Forwards section of its pane.
- Importing an ssh config brings `LocalForward` lines across as forwards.
- The window opens where you left it, and the sidebar keeps the width you drag it to.
- Terminal sessions use patchbay's colours, not the raw VT palette. Links are clickable.
- Clicking a device you already have open goes to that tab.
- patchbay updates itself. It asks once when the window opens, downloads only
  when you say so, and never restarts you: the new version is put in place and
  the button becomes `Restart`, which is a separate click because it takes your
  live sessions with it. Settings ▸ Updates has the switch and a check you can
  run yourself; the macOS menu has one too.
- The disk image is an installer now, with the app and Applications where you
  expect them, and it ejects itself once you've dragged the app out.
- Everything downloaded is verified against a key built into the app before a
  single file is replaced.
