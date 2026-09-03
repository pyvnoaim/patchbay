# Changelog

Written for someone about to install, not for someone reading the diff: what
changed for them, in plain sentences. Newest first.

This file is the source. `npm run bump` stamps `## Unreleased` with the version
and the date, the release workflow lifts that section into the GitHub release,
and from there the app shows it before it installs anything and the site reads
it back. Nothing is retyped anywhere.

## Unreleased

- **Broadcast to marked devices.** Pick out several boxes, right-click, hit Broadcast:
  every device opens as its own pane in one tab, and one keystroke reaches all of them.
  Click a pane to focus it and one keystroke goes there instead; the tab's broadcast
  toggle turns fan-out on and off without closing anything. Marks a NAS's web UI along
  with the ssh boxes? The web ones are left out and named.
- A bulk-action bar appears when you mark two or more devices: move them, broadcast to
  them, or delete them without hunting for the right-click menu.
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
- `patchbay://web-01` in a runbook or an alert opens that device. macOS for now.
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
- ⌘-click and shift-click pick out several devices at once, to delete them or
  move them to another space in one go.
- A device's forwards can be held open on their own, without a session, from the
  Forwards section of its pane.
- Importing an ssh config brings `LocalForward` lines across as forwards.
- A space has a page of its own now: which file it is, and what its sync is doing.
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
