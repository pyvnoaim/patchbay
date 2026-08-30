# patchbay

SSH connection manager: one TOML file, exec `ssh`. A deliberately tiny answer to
Royal TS — no Electron, no sync service, no stored credentials.

- `src/patchbay.ts` — config load, `[defaults]` inheritance, jump-chain walk, name resolve. All the logic worth testing lives here.
- `src/cli.ts` — arg dispatch and process spawning. Keep it dumb.
- `test/patchbay.test.ts` — `node:test` + `assert`. `npm test`.

## Non-obvious

- **No build step.** Node runs the `.ts` directly; the flags live in the `src/cli.ts` shebang (`--experimental-strip-types`, needed until Node 23.6). So: no enums, no namespaces, no parameter properties, and relative imports must end in `.ts`.
- **`$PATCHBAY_CONFIG`** overrides the config path — that's how you exercise the CLI without touching `~/.config`.
- Jump chains resolve by walking `jump` until it hits a non-jack name (passed through raw) or runs out. The cycle guard is load-bearing; don't drop it.

## Scope

We shell out to `/usr/bin/ssh` on purpose — the agent, `~/.ssh/config` and
`known_hosts` come free. Do not add an ssh2 client, a GUI, or a credential store.
RDP, when it lands, hands off to the system client; embedding FreeRDP is the
thing this project exists to avoid.
