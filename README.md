<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
  <img src="assets/logo-light.svg" alt="patchbay" width="272">
</picture>

**Every host, one jack away.**

A connection manager that is a TOML file and a `ssh` exec. No Electron, no sync
service, no license key, no crown.

```
bay                   pick a jack (fzf) or list them
bay prod-web          connect - a unique substring is enough
bay prod-web -v       any flag that isn't ours goes to ssh
bay prod-web -n       print the ssh command instead of running it
bay prod-web -- uptime  run one command instead of a shell
bay ls [filter]       list jacks, filtered by name or folder
bay ls --names        names alone; --space <space> for one space
bay edit              open the config
bay import [file]     print TOML for the hosts in your ssh config
bay completion zsh    a snippet for zsh, bash or fish
```

The CLI prints the TOML for you to check and paste. In the app it's the same list with
tick boxes - right-click the device list, or the button on the empty state.

The window adds what a terminal can't: sessions, remote desktop, a device's web UI and
its files, all as tabs; a folder tree; and an optional shared list for a team. Everything
it knows is still those TOML files, so `bay` and the window never disagree.


## Install

Download it from [Releases](https://github.com/pyvnoaim/patchbay/releases) - macOS,
Linux and Windows. `bay` is inside the app: Settings → Config file → **Install the bay
command** links it onto your PATH, so the terminal and the window are the same build
and updating one updates the other.

For tab completion, put `eval "$(bay completion zsh)"` in your `~/.zshrc` - `bash` and
`fish` are there too, and the names come from your config every time, so a snippet never
goes stale.

You need an `ssh` on your PATH. On Windows that means the OpenSSH Client, which ships
with Windows 10/11 - if it's missing, enable it under Settings → Apps → Optional
Features. `fzf` is optional everywhere; without it, bare `bay` just lists.

## Config

`~/.config/patchbay/patchbay.toml`, or `%APPDATA%\patchbay\patchbay.toml` on Windows -
or wherever `$PATCHBAY_CONFIG` points. `bay edit` creates it.

That file is your own list. **A space is another one of these**, in `spaces/` beside it,
same format, shown under its own heading in the sidebar - `bay edit <space>` opens one.
Keep a customer's machines apart from your own, or share one with a team without ever
handing over the rest.

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
forward = ["8080:localhost:80"]
folders = ["prod", "web"]
desc = "main web box"
```

Jump chains resolve by walking `jump` until it runs out, so `db → web → bastion`
becomes one `ssh -J` list. Loops throw instead of hanging.

## Development

```sh
npm run dev          # the app window, against dev/patchbay.toml
npm run build        # patchbay.app / .exe / .deb, with `bay` inside it
npm test             # all three suites
```

`npm run dev` needs Rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`) -
no need to restart your shell, the dev script finds `~/.cargo/bin` itself. The first
build compiles a few hundred crates and takes minutes; after that it's seconds.

`npm run cli -- <args>` runs the CLI against the same sample config, so you can poke
at it without touching `~/.config`:

```sh
npm run cli -- ls            # the sample jacks
npm run cli -- db -n         # print the ssh command, don't run it
npm run cli -- local         # actually connect, if you have sshd running
```

`-n` works on the installed `bay` too - it's the fastest way to see what a jump chain
expands to. The shipped `bay` is `src-tauri/src/bin/bay.rs`, built by `npm run build`
and by `cargo run --bin bay`; `npm run cli` runs the TypeScript it was ported from.

The app opens sessions in its own window - the system `ssh` on a real pty, with the
same argv the CLI builds, streamed to xterm.js. Your agent, `~/.ssh/config` and
host-key prompts all still work, because it *is* your ssh. "Open in Terminal" is on
the context menu when you'd rather have your own terminal.

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
into a folder or to download a file, and drop files onto it to upload.

Same connection as everything else - your agent, your `~/.ssh/config`, your jump chain.
Repeated listings share one ssh session, so only the first one authenticates.

## Teams

**A team is a space with a server behind it.** Make a space, put the devices you want
to share in it, then Settings → Spaces → **Share with a team** - that hands you a code,
and anyone who types it into their own window gets that space. Joining one writes a new
file and touches nothing you already had; your own list is never read, never uploaded,
and never replaced.

Run the server yourself: `npm run server`, or the `patchbay-server` binary anywhere that
has a disk. The window syncs when it gets focus and after every edit; `bay` reads
whatever that left on disk, so the CLI never waits on a server.

Two people adding two devices is not a disagreement - those merge. Only the same field
on both sides needs an answer, and then the window asks which one wins; whichever loses
is kept as `<space>.toml.bak`. There are no accounts: the code *is* the credential, so
anyone who has it has that space.

`/team` is plain HTTP - `GET` with an `ETag`, `PUT` with `If-Match` - so a space can
point at any file over HTTP instead. Leave the code blank and give the address of a TOML
file someone publishes, a raw git URL included, and you get a read-only copy of it.

Three seats are free; past that everyone can still read the list, and writing asks you
to pay. Nothing is stored anywhere unless you point patchbay at a server yourself.

Nothing in a config is ever executed: whatever a colleague puts in a space, the most it
can do here is produce an `ssh` command line. That is what makes joining one safe.

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
