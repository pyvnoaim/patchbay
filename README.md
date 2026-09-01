<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
  <img src="assets/logo-light.svg" alt="patchbay" width="272">
</picture>

**Every host, one jack away.**

A connection manager that is one TOML file and a `ssh` exec. No Electron, no sync
service, no license key, no crown.

```
bay                   pick a jack (fzf) or list them
bay prod-web          connect — a unique substring is enough
bay prod-web -n       print the ssh command instead of running it
bay prod-web -- uptime  run one command instead of a shell
bay ls [filter]       list jacks, filtered by name or folder
bay edit              open the config
bay import [file]     print TOML for the hosts in your ssh config
```

The CLI prints the TOML for you to check and paste. In the app it's the same list with
tick boxes — right-click the device list, or the button on the empty state.


## Install

Needs Node ≥ 22.6 (it runs the TypeScript directly — there is no build step) and an
`ssh` on your PATH. macOS, Linux and Windows.

```sh
npm install -g patchbay
```

On Windows that means the OpenSSH Client, which ships with Windows 10/11 — if it's
missing, enable it under Settings → Apps → Optional Features. `fzf` is optional
everywhere; without it, bare `bay` just lists.

## Config

`~/.config/patchbay/patchbay.toml`, or `%APPDATA%\patchbay\patchbay.toml` on Windows —
or wherever `$PATCHBAY_CONFIG` points. `bay edit` creates it.

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
npm run build        # patchbay.app / .exe / .deb
npm test             # both suites
```

`npm run dev` needs Rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`) —
no need to restart your shell, the dev script finds `~/.cargo/bin` itself. The first
build compiles a few hundred crates and takes minutes; after that it's seconds.

`npm run cli -- <args>` runs the CLI against the same sample config, so you can poke
at it without touching `~/.config`:

```sh
npm run cli -- ls            # the sample jacks
npm run cli -- db -n         # print the ssh command, don't run it
npm run cli -- local         # actually connect, if you have sshd running
```

`-n` works on the installed `bay` too — it's the fastest way to see what a jump
chain expands to. To try the real command, `npm link`, then `bay ls`.

The app opens sessions in its own window — the system `ssh` on a real pty, with the
same argv the CLI builds, streamed to xterm.js. Your agent, `~/.ssh/config` and
host-key prompts all still work, because it *is* your ssh. "Open in Terminal" is on
the context menu when you'd rather have your own terminal.

## Folders, VPNs and web UIs

A folder with slashes (`prod/eu/web`) nests in the sidebar, and `folders` is a list,
so a device can sit in several branches at once. A device says how it is reached —
`ssh` (on by default), `rdp = <port>`, `url` — and `primary` picks what Enter and a
double-click do. A folder can carry a VPN, which is how one-customer-per-folder
works — flip the switch, or let it come up on its own when you connect.

```toml
[vpn.acme]
provider = "tailscale"          # or tunnelblick / wireguard / custom
profile  = "acme"

[jack.acme-nas]
host = "10.80.0.20"
os   = "synology"               # picks the icon, and its colour
url  = "https://10.80.0.20:5001"   # opens in your browser
folders = ["acme/prod"]

[jack.acme-dc]
host = "10.80.0.5"
rdp  = 3389                     # remote desktop, in a tab
ssh  = false
primary = "rdp"                 # what Enter opens
folders = ["acme/prod"]
```

A `custom` VPN runs whatever `up`/`down`/`check` you give it, so treat a config
someone sends you the way you'd treat their shell script.

## Teams

One shared list, on a server you run — `npm run server`, or the `patchbay-server`
binary anywhere that has a disk. Settings → Team → **Create a team** hands you a code;
anyone who types that code into their own window has the same list.

The shared thing is the config file itself, so there is nothing new to learn: devices,
folders, VPNs and colours are the team's, while the preferences above them stay on your
machine. The window syncs when it gets focus and after every edit; `bay` reads whatever
that left on disk, so the CLI never waits on a server.

There is no merge and no account. The code *is* the credential — anyone who has it has
the list — and if your list and the team's have both moved since they last agreed, the
window says so and asks which one wins. Whichever loses is kept as `patchbay.toml.bak`.

Three seats are free; past that everyone can still read the list, and writing asks you
to pay. Nothing is stored anywhere unless you point patchbay at a server yourself.

Worth knowing before you join one: a `[vpn]` block runs the commands written in it, and
joining a team means those arrive from your colleagues rather than from you. Join teams
you'd trust with a shell script, which is the same rule as any config someone sends you.

## What it deliberately isn't

Real credentials live in your ssh agent and your existing keys — patchbay stores
none, so there's nothing here to leak or sync. There is no SSH implementation in
here and never will be: a session is `/usr/bin/ssh` on a pty, in a tab.

Remote desktop is a tab too. That one decodes RDP itself — IronRDP, pure Rust,
painted onto a canvas — so there is still no FreeRDP and no embedded graphics
toolkit, which was always the actual objection. "Remote desktop in system client"
hands a `.rdp` file to mstsc / Windows App / xfreerdp if you'd rather. VNC isn't in.
