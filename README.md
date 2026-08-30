# patchbay

**Every host, one jack away.**

A connection manager that is one TOML file and a `ssh` exec. No Electron, no sync
service, no license key, no crown.

```
bay                   pick a jack (fzf) or list them
bay prod-web          connect — a unique substring is enough
bay prod-web -n       print the ssh command instead of running it
bay prod-web -- uptime  run one command instead of a shell
bay ls [filter]       list jacks, filtered by name or tag
bay edit              open the config
```

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
tags = ["prod"]

[jack.prod-web]
host = "10.0.0.4"
user = "deploy"
key  = "~/.ssh/prod"
jump = "bastion"                 # another jack name, or a raw user@host
forward = ["8080:localhost:80"]
tags = ["prod", "web"]
desc = "main web box"
```

Jump chains resolve by walking `jump` until it runs out, so `db → web → bastion`
becomes one `ssh -J` list. Loops throw instead of hanging.

## Development

```sh
npm run dev          # the app window, against patchbay.dev.toml
npm run build        # patchbay.app / .exe / .deb
npm test             # both suites
```

`npm run dev` needs Rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
The first build compiles a few hundred crates and takes minutes; after that it's seconds.

The CLI needs no toolchain at all — `npm run cli -- <args>` runs it against the same
sample config, so you can poke at it without touching `~/.config`:

```sh
npm run cli -- ls            # the sample jacks
npm run cli -- db -n         # print the ssh command, don't run it
npm run cli -- local         # actually connect, if you have sshd running
```

`-n` works on the installed `bay` too — it's the fastest way to see what a jump
chain expands to. To try the real command, `npm link`, then `bay ls`.

The app is a launcher, not a client: it renders the list and hands the `ssh` command
to your real terminal. Nothing is embedded, so there's no terminal emulator and no
second SSH implementation to keep alive.

## What it deliberately isn't

Real credentials live in your ssh agent and your existing keys — patchbay stores
none, so there's nothing here to leak or sync. RDP and VNC aren't in yet; when
they land, RDP will hand off to the system client before it ever embeds FreeRDP.
