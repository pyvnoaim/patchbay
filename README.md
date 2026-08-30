# patchbay

**Every host, one jack away.**

A connection manager that is one TOML file and a `ssh` exec. No Electron, no sync
service, no license key, no crown.

```
bay                   pick a jack (fzf) or list them
bay prod-web          connect — a unique substring is enough
bay prod-web -- uptime  run one command instead of a shell
bay ls [filter]       list jacks, filtered by name or tag
bay edit              open the config
```

## Install

Needs Node ≥ 22.6 (it runs the TypeScript directly — there is no build step).

```sh
npm install -g patchbay
```

## Config

`~/.config/patchbay/patchbay.toml`, or wherever `$PATCHBAY_CONFIG` points.

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

## What it deliberately isn't

Real credentials live in your ssh agent and your existing keys — patchbay stores
none, so there's nothing here to leak or sync. RDP and VNC aren't in yet; when
they land, RDP will hand off to the system client before it ever embeds FreeRDP.
