<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
  <img src="assets/logo-light.svg" alt="patchbay" width="272">
</picture>

**Every host, one jack away.**

A connection manager that is a TOML file and a `ssh` exec. No Electron, no sync
service, no license key, no crown.

Your machines live in one file you can read, hand-edit and keep in a repo. The window
opens them: sessions, remote desktop, a device's web UI and its files, all as tabs,
under a folder tree. It stores no credentials, because your ssh agent and
`~/.ssh/config` already hold them.

Already have a list? Settings → Import reads `~/.ssh/config` or a Royal TS `.rtsz`
document and shows what it found with tick boxes. Nothing is written until you tick,
and passwords never come across.


## Install

Download it from [Releases](https://github.com/pyvnoaim/patchbay/releases) - macOS,
Linux and Windows.

You need an `ssh` on your PATH. On Windows that means the OpenSSH Client, which ships
with Windows 10/11 - if it's missing, enable it under Settings → Apps → Optional
Features.

## Config

`~/.config/patchbay/patchbay.toml`, or `%APPDATA%\patchbay\patchbay.toml` on Windows -
or wherever `$PATCHBAY_CONFIG` points. The window creates it on first run, and
Settings → Config file opens it in your editor.

That one file is the whole list. Grouping is the `folders` list on each device, and a
folder can carry a note for whoever arrives at that customer next.

```toml
[defaults]                       # inherited by every jack; the jack wins
user = "root"

[jack.bastion]
host = "bastion.example"
port = 2222
folders = ["prod"]

[jack.prod-web]
host = "10.0.0.4"
user = "deploy"
key  = "~/.ssh/prod"
jump = "bastion"                 # another jack name, or a raw user@host
forward = ["8080:localhost:80", "-D 1080"]   # -L unless it says -R or -D
folders = ["prod", "web"]
desc = "main web box"

[folder.prod]
note = "change window is Tuesday 22:00; ask ops before touching bastion"
```

Jump chains resolve by walking `jump` until it runs out, so `db → web → bastion`
becomes one `ssh -J` list. Loops throw instead of hanging.

Nothing in the file is ever executed as written. The most it can produce is an `ssh`
command line, and the window shows it before it runs.

## Development

```sh
npm run dev          # the app window, against dev/patchbay.toml
npm run build        # patchbay.app / .exe / .deb
npm test             # the app and the server
```

`npm run dev` needs Rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`) -
no need to restart your shell, the dev script finds `~/.cargo/bin` itself. The first
build compiles a few hundred crates and takes minutes; after that it's seconds.

Everything local runs against `dev/patchbay.toml`, a copy of the tracked sample, so you
never touch your own `~/.config` while working on it.

The app opens sessions in its own window - the system `ssh` on a real pty, with the argv
`patchbay.rs` builds, streamed to xterm.js. Your agent, `~/.ssh/config` and
host-key prompts all still work, because it *is* your ssh. "Open in Terminal" is on
the context menu when you'd rather have your own terminal.

## The rest of your machine

Settings ▸ Devices can write your list to `~/.ssh/patchbay.conf` and add one `Include`
line to `~/.ssh/config`. Then `ssh web-01` works in any terminal, and so do `scp`,
`rsync`, Ansible and VS Code Remote - with the jump chains patchbay resolved, as
`ProxyJump`. Your own config is never rewritten: one line goes in at the top, a name
you already define is left to you, and turning it off takes the line and the file away.

It goes the other way too: `patchbay://web-01` in a runbook or an alert opens that
device. A link carries a device *name*, resolved against your own config - never an
address, so a link can't point you at a machine you don't have. macOS for now.

## Folders and web UIs

A folder with slashes (`prod/eu/web`) nests in the sidebar, and `folders` is a list,
so a device can sit in several branches at once. A device says how it is reached - `ssh`
(on by default), `rdp = <port>`, `vnc = <port>` or `url`, one of them - and that is what
Enter and a double-click open.

```toml
[jack.acme-nas]
host = "10.80.0.20"
os   = "synology"               # picks the icon, and its colour
url  = "https://10.80.0.20:5001"   # opens in a tab
folders = ["acme/prod"]

[jack.acme-dc]
host = "10.80.0.5"
rdp  = 3389                     # remote desktop, in a tab
ssh  = false
folders = ["acme/prod"]
```

A device's web UI opens in a tab, beside your terminals. Appliances are usually reached
by IP, so their certificate names something else and the page would be blank with no
explanation - patchbay says which certificate and why, and can hand it to your system's
trust store the way the browser's **Always trust** does. Plain `http` works too, but
only to your own network; a public one goes to the browser.

## Files

`ssh` ships `sftp`, so patchbay uses it. Right-click a device and **Browse files**, or
the folder button in the detail pane: a tab listing the remote side, double-click to go
into a folder or to download a file, and drop files onto it to upload. Right-click a
file to rename or delete it, or **Edit here** to open it in whatever this machine opens
it with - saving puts it straight back.

Same connection as everything else - your agent, your `~/.ssh/config`, your jump chain.
Repeated listings share one ssh session, so only the first one authenticates.

## In the window

- **Map** beside the list groups devices by the route to them, so a bastion and
  everything behind it sit together however they are filed. A bastion that isn't
  answering says so once.
- **Marks.** ⌘-click and shift-click pick out several devices; a dock appears to move,
  delete or **broadcast** to them - every ssh mark as a pane in one tab, one keystroke
  reaching all of them.
- **Find** in a session: ⌘F, or Ctrl+Shift+F elsewhere, searches that terminal's scrollback.
- **Session logs**, off by default, append each session's output to `logs/` beside the config.
- **Updates** are checked once at launch, downloaded only when you say so, and never
  restart you: the button becomes `Restart`, a second click, because it takes your
  sessions with it.

## What it deliberately isn't

Real credentials live in your ssh agent and your existing keys - patchbay stores
none, so there's nothing here to leak or sync. There is no SSH implementation in
here and never will be: a session is `/usr/bin/ssh` on a pty, in a tab.

Remote desktop is a tab too. That one decodes RDP itself - IronRDP, pure Rust,
painted onto a canvas - so there is still no FreeRDP and no embedded graphics
toolkit, which was always the actual objection. "Remote desktop in system client"
hands a `.rdp` file to mstsc / Windows App / xfreerdp if you'd rather.

VNC is a handoff and only a handoff: `vnc = 5900` opens `vnc://` with whatever viewer
the machine already has - Screen Sharing on macOS - over the same forwarded port when
the device is behind a bastion. A second protocol decoder in here would have to earn
its place, and one screen-sharing tab already exists.

Files are `sftp`, the binary, not a library. A web UI is a webview showing the device's
own page, not something we render. The pattern holds: patchbay knows where your machines
are and what to run - the running is someone else's job.
