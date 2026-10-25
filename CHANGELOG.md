# Changelog

Written for someone about to install, not for someone reading the diff: what
changed for them, in plain sentences. Newest first.

This file is the source. `npm run bump` stamps `## Unreleased` with the version
and the date, the release workflow lifts that section into the GitHub release,
and from there the app shows it before it installs anything and the site reads
it back. Nothing is retyped anywhere.

## 0.1.59 - 2026-10-26

- Settings ▸ Licence takes a team's company licence. Nothing needs one yet.
- Telnet and serial consoles open in a tab, and Royal TS telnet connections now import.
- Split a tab: ⌘D opens a device beside the one in front; drag the line between panes to resize.
- Works with a screen reader, says a device's status in words, and respects Reduce Motion.
- Remote desktop shows Windows' own pointer: the text cursor over text, the resize arrows on a window's edge.
- Remote desktop plays the desktop's sound on this machine.
- VNC opens in a tab, like remote desktop: no viewer to install on Windows or Linux. A Mac's Screen Sharing, or any server patchbay can't sign in to, still opens in your VNC viewer.
- New licence: free for one person, at work too. A team on a shared list will need a company licence, €249 a year, and until it goes on sale a shared list stays free.

## 0.1.58 - 2026-10-25

- A shell prints large output much faster, and accented letters, `€` or box drawing no longer turn into `�` now and then.
- A device's web page open in a tab can no longer read what a shell or a remote desktop in another tab is showing.
- Coming back to the window no longer rereads the whole list unless it changed, which is quicker on a shared list over the network.

## 0.1.57 - 2026-10-24

- Remote desktop in a tab is much smoother: scrolling or dragging a window draws several times as many frames.
- Remote desktop speaks the graphics pipeline Windows uses with its own client, with H.264 decoded by your Mac's video hardware: less traffic, sharper text, and less CPU.
- A snippet can leave a blank, `{?}` or `{?unit}`, that you are asked to fill in when you pick it: `journalctl -u {?unit} -f`.
- The first time you connect to a team device, patchbay offers to share its host key with the team.
- Adding your devices to your ssh config can now add your own (Mine) instead of the team's, picked in Settings ▸ Devices.
- The sidebar's folders no longer draw vertical guide lines; the indent alone shows what is inside.
- Dropping files on a remote desktop tab is gone; copy and paste moves files both ways, on every platform.
- A submenu with no room on either side of the menu now opens over it instead of off the window.

## 0.1.54 - 2026-10-23

- Older Windows servers (2008 R2) that refuse the built-in remote desktop now open in your Remote Desktop app instead of failing with "connection reset by peer".
- Retrying a remote desktop that failed replaces its dead tab instead of opening a second one.

## 0.1.53 - 2026-10-23

- The Copy submenu is solid now, so a terminal behind it no longer shows through.

## 0.1.52 - 2026-10-22

- The remote desktop sign-in no longer repeats your saved username in a wide bar under the field; other saved accounts show as small chips.

## 0.1.51 - 2026-10-22

- Trust it now works for a device whose certificate has expired. Until now the tab stayed blank however often you trusted it.
- The trust question shows the certificate's name, issuer, expiry and fingerprint as tidy rows, and an expired one in red.
- A web tab that is still loading shows the device and its address in the middle of the pane, not a line of text in the corner.
- Copy on a device's menu opens its submenu when you hover over it.

## 0.1.50 - 2026-10-22

- A folder with one device in it shows as a folder, in the sidebar and the overview, so its name is no longer hidden behind the device's.

## 0.1.49 - 2026-10-22

- The devices and folders in a folder's overview carry their icon, the same one the sidebar shows.

## 0.1.48 - 2026-10-22

- A folder's pane sets the user, port, key and jump host for every device in it, and its right-click menu goes straight there.
- Settings ▸ Defaults is gone: folders do that job. A `[defaults]` already in a file still applies.
- The New button left the toolbar; ⌘N and the folder menu still add a device.
- Snippets can say `{host}`, `{user}`, `{port}` and `{name}`, filled in from each device, and a Snippets button on a shell tab lists them.
- Settings ▸ Team shows where the shared list is, drive first, instead of a path cut off before the file.
- The file that shows who is connected is never written through a linked folder either.
- A device's pane sits in the middle of a wide window instead of against the sidebar.

## 0.1.47 - 2026-10-22

- The file that shows who is connected is never written through a link someone else put on the share.

## 0.1.46 - 2026-10-22

- A web tab that is slow to load, like a NAS waking up, says it is opening instead of showing a blank white pane.
- Showing who is connected no longer leaves a file in a synced share's bin every time patchbay quits.

## 0.1.45 - 2026-10-22

- Tabs can be dragged into any order, and they come back in that order on the next launch.

## 0.1.44 - 2026-10-22

- A closed ssh tab offers `r`: wait until the device answers again, then reconnect. Handy after a reboot.
- A device's menu has Copy…, with the host, user@host, web address, MAC and ssh command.

## 0.1.43 - 2026-10-22

- The Team row in the sidebar names the drive the shared list is on, such as "Systemhaus" or "Google Drive", instead of a path cut off halfway. Hover it for the path.

## 0.1.42 - 2026-10-22

- A folder's overview uses the whole width of the window instead of stopping halfway.

## 0.1.41 - 2026-10-22

- In the light theme, selected text and highlighted text (a paste, for one) in a session are readable again instead of dark on dark.

## 0.1.40 - 2026-10-21

- **Windows is back.** An installer again, `patchbay_x64-setup.exe`; SmartScreen asks once, because it is not signed.

## 0.1.39 - 2026-10-20

- A device's route is drawn as a line from this machine to it, each jump a stop on the way.
- The device page has its icon in a tile, and its buttons stay beside the name in a wide window.
- Section labels are plain words instead of capitals, and only non-ssh devices carry a kind icon in the sidebar.
- On Windows and Linux the sidebar no longer starts below an empty band.

## 0.1.38 - 2026-10-20

- An import asks whether the devices go in Mine or Team, instead of landing in whichever was open.
- With a shared list, Settings ▸ Defaults edits your own defaults too, not only the team's.

## 0.1.37 - 2026-10-15

- Trusting a device's self-signed certificate works on macOS again, instead of failing with "one or more parameters were not valid".
- Securepoint and firewall devices get a firewall icon, and a kind like "Router" now matches "RouterOS" too.

## 0.1.36 - 2026-10-14

- The sidebar draws a guide line under each folder, so what is inside it is easier to tell apart.
- The folder page's own "Device" button is gone; New in the top bar already files into the folder.
- "Out of reach" no longer breaks over two lines.

## 0.1.35 - 2026-10-12

- A folder's notes sit at the top of its page, beside what answers from this network.
- The user, port, key and jump boxes are gone from the folder page; a folder's settings in the file still apply.
- Clearing a folder's note no longer removes the settings written beside it.

## 0.1.34 - 2026-10-11

- The Ping, Trace and key buttons beside a session wrap instead of running off the edge of the pane.

## 0.1.33 - 2026-10-11

- Every folder in the sidebar has a dot: green when something in it answers from this network.
- A device with a session open shows as up right away, and a new device is checked as soon as it is saved.
- The folder box only suggests folders from the vault the device is in.
- The triangle beside a folder is a much bigger target.

## 0.1.32 - 2026-10-11

- A vault or folder page shows how much of it answers from this network and what is in it, as tiles you can click into.
- A folder with a single device has a triangle like any other, so it can be unfolded.
- A device that doesn't answer from here has a grey dot instead of a red one: usually that is a VPN that is off, not a fault.

## 0.1.31 - 2026-10-11

- A folder with a single device is named after that device in the sidebar, so the row says what a click opens.

## 0.1.30 - 2026-10-11

- "Find the break" no longer uses the team's recorded host keys when it asks a hop: only the keys this machine already trusts count there.

## 0.1.29 - 2026-10-11

- **Find the break.** On a device behind a jump host, patchbay asks each hop in turn whether the next one answers and marks the link that doesn't on the route.
- **Host keys the team has seen.** Share the key your machine trusts for a team device, and everyone else's ssh checks against it without asking. A key that changes is refused for all of you.
- **Who is connected.** On a shared list, everyone sees which team devices a colleague has a session on right now, and they see yours. Needs no server, and never says anything about your own devices.

## 0.1.28 - 2026-10-11

- **Snippets.** Lines you type often, kept in Settings ▸ Snippets and picked in Search while a shell is in front. patchbay types the line at the prompt and never presses Enter. Into a broadcast it goes to every pane.
- A snippet is yours or the team's, like a device. New ones are yours until you say otherwise.
- **Mine and Team.** With a shared list set, your own devices show beside the team's as a second vault only this machine sees. Both can have a `web`; a device of yours can jump through the team's bastion, never the other way round.
- Move a device between the two from its menu, its detail or the sheet's "Who can see it?".
- When the shared list is away, your own devices still open and the sidebar says why the team's are missing.
- Starting a team list now moves your devices into it. If an earlier one left copies behind, Settings ▸ One shared list removes them.
- **The sidebar is the list.** Folders unfold to their devices, a folder with one device shows it on its own row, and whatever you select is described across the whole right-hand side.
- A session gets the full width; the Details button brings the device's pane back beside it.
- Right-clicking in the sidebar no longer blanks the web page beside it.
- The Sort menu is gone: folders are A to Z, devices in the order the file has them.
- Bitwarden works with a server of your own: Settings ▸ Bitwarden ▸ Your own server.
- Renaming or deleting a folder no longer leaves two devices of one name in the same place; it says which one is in the way.
- A remote desktop moved to a screen with another sharpness redraws at the new one.
- Quitting closes the connections the file browser kept open, instead of leaving them for a minute.

## 0.1.27 - 2026-10-06

- **Install my key** on an ssh device makes you a key if you have none and puts it on the device, asking the password once in a tab. It uses the device's `key` path, so a shared list can name one for everyone.

## 0.1.26 - 2026-10-02

- A remote desktop can remember more than one account. The sign-in offers every saved one; pick it and press Enter.
- Editing a device no longer copies the user, port, key or jump it inherits from its folder onto the device itself.
- The folder pane's User/Port/Key/Jump fields fit in a narrow window.

## 0.1.25 - 2026-09-30

- On a Mac, ⌘C, ⌘X, ⌘V, ⌘A and ⌘Z in a remote desktop do what Ctrl does in Windows, so copy and paste work.
- A remote desktop's sign-in, asked again after a failed attempt, keeps the cursor in the password field.

## 0.1.24 - 2026-09-30

- A remote desktop can remember its password: tick Remember on the sign-in and it goes in your system keychain, never in the list. Forget one from the right-click menu, or all of them in Settings.

## 0.1.23 - 2026-09-30

- A remote desktop no longer falls further and further behind on a busy screen: it skips to the latest picture, and typing on a quiet one answers sooner.
- A remote desktop fills its tab at the right sharpness, even when the server first answers with another size.
- Trackpad scrolling in a remote desktop follows your fingers instead of jumping a page per flick.
- Remote desktops show a plain background instead of the wallpaper, so closing a big window no longer dissolves into pixels.
- Remote desktops and web pages sit flush in their tab, without a gap along the edges.

## 0.1.22 - 2026-09-25

- The key on a web tab opens Bitwarden's sign-in when the vault is signed out or locked, instead of doing nothing.
- patchbay opens centered at its default size every time, rather than wherever it was last left.
- In a narrow window the detail pane no longer drops under the sidebar and squeezes a session into a strip.

## 0.1.21 - 2026-09-25

- Fixes 0.1.20 failing to start with an error when the list has more than one folder.

## 0.1.20 - 2026-09-25

- Two devices can share a name when they're in different folders.
- The key on a web tab opens Bitwarden's unlock screen when the vault is locked, instead of doing nothing.

## 0.1.19 - 2026-09-24

- Remote desktops draw faster, most of all when much of the screen changes at once.
- The remote desktop sign-in has a Vault button when Bitwarden is on: copy the password there and paste it in.

## 0.1.18 - 2026-09-24

- Copy and paste files between this machine and a remote desktop, both ways, as you would text. In the remote desktop paste with Ctrl+V.
- Remote desktops can take files: drop them on the tab and they appear as `\\tsclient\transfer` on the far end. Whatever it saves there lands in `Downloads/patchbay`. macOS and Linux.

## 0.1.16 - 2026-09-23

- Once a ping finishes, press `c` in its tab to keep pinging until you press ^C.

## 0.1.15 - 2026-09-23

- Proxmox and Synology tabs no longer ask you to trust them again after you switch away and come back.

## 0.1.14 - 2026-09-22

- A device you trusted that is restarting now says it isn't answering, instead of asking you to trust it again.
- Remote desktops behind a bastion no longer refuse each other's certificates, and two machines on one address with different ports are told apart.
- When a remote desktop's certificate changes, patchbay shows the new one and offers to trust it, instead of sending you to edit a file.

## 0.1.13 - 2026-09-22

- Proxmox no longer shows "opened nothing" over a working page after you click around in it.

## 0.1.12 - 2026-09-21

- Messages at the foot of the window stack instead of replacing each other, so an error no longer takes a delete's Undo away.
- Deleting a second device while the Undo pill is still up adds it to the same Undo, so both come back.

## 0.1.11 - 2026-09-21

- A web tab's popups open. Proxmox's Upgrade shell and a VM's console used to do nothing when clicked.

## 0.1.10 - 2026-09-18

- Trusting a device's certificate now survives its firmware update: the appliance's own signing certificate is trusted, not the one it replaces on every update. The sheet says when that is what you are trusting.
- A web tab that stays blank now says so even for a device you have already trusted, instead of sitting empty.
- A folder can lend its devices a user, port, key and jump host, so a customer's bastion is written once instead of on every device behind it. Select the folder to set them.
- Wake on LAN: give a device a `mac` and its pane has a Wake button.
- Search and the palette match a device's url as well as its host.
- A device's pane names the folders it is in, and marks a user, port, key or jump host it inherited rather than set itself.

## 0.1.9 - 2026-09-16

- A device with a long name connects again on macOS instead of exiting with a socket error.
- No Windows build this release; macOS and Linux only for now.

## 0.1.8 - 2026-09-16

- Settings ▸ Team has New… to start a shared list in a folder and Open… to join one.
- Shorter text in Settings ▸ Team and Bitwarden.

## 0.1.7 - 2026-09-16

- Windows is now only the setup `.exe`; the `.msi` is no longer published.

## 0.1.6 - 2026-09-15

- Scrollbars match the theme instead of painting a light grey strip down a dark list.

## 0.1.5 - 2026-09-15

- A key beside Search opens Bitwarden anywhere, so a password is one click to read
  whether or not a web tab is open.
- Copy in the Bitwarden popup now actually copies.
- An update found from Settings shows its Install button once the sheet is closed,
  instead of sitting behind it.

## 0.1.4 - 2026-09-15

- A session's last line is no longer cut in half by the bottom of the window.
- Sending the window to a screen with a different scale no longer leaves the page
  drawn at half the window.
- The detail pane steps aside sooner, so the header's buttons stop running over it
  in a narrow window.

## 0.1.3 - 2026-09-15

- Remote desktop: opening one no longer renegotiates the screen a moment after it
  appears, which is what made the first few seconds look like it was calibrating.
- Remote desktop: the mouse is sent once a frame rather than once an event, so a drag
  is a fraction of the traffic.

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
