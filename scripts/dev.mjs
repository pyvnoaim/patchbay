// The one entry point for running patchbay locally.
//
//   node scripts/dev.mjs          the app window
//   node scripts/dev.mjs build    a release bundle
//   node scripts/dev.mjs cargo …  cargo, with PATH sorted out
//
// It exists because two things need fixing before the app starts, and npm scripts
// can't do them portably: pointing at the sample config (an env-var prefix doesn't
// work in cmd.exe) and finding cargo (rustup edits your shell profile, so a terminal
// opened before you installed it has no cargo on PATH).
import { spawn } from "node:child_process";
import { copyFileSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

const [mode = "dev", ...rest] = process.argv.slice(2);

// `tauri dev` runs the binary with src-tauri/ as its cwd, so this has to be absolute.
const DEV_CONFIG = resolve("dev/patchbay.toml");

// The dev config isn't tracked - it fills up with your own machines, and a NAS address
// belongs in a public repo about as much as a password does. Seed it from the example
// so a fresh clone still has something to open.
if (!existsSync(DEV_CONFIG)) copyFileSync(resolve("dev/patchbay.example.toml"), DEV_CONFIG);

const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) process.env.PATH = `${cargoBin}${delimiter}${process.env.PATH}`;

// The updater signs its artifact or prints an error at the end of every build, which
// reads like a failed one. The key never lives in the repo; CI passes the key itself
// through TAURI_SIGNING_PRIVATE_KEY, and a machine that has neither just builds a
// bundle nobody can update from, which is what a local build is anyway. The one
// variable takes a path or the key itself - there is no `_PATH` twin, and setting one
// was the same error at the end of every build. The key is password-protected, so
// a build here still wants TAURI_SIGNING_PRIVATE_KEY_PASSWORD in the environment.
const signingKey = join(homedir(), ".tauri", "patchbay.key");
if (existsSync(signingKey) && !process.env.TAURI_SIGNING_PRIVATE_KEY) {
  process.env.TAURI_SIGNING_PRIVATE_KEY = signingKey;
}

const run = (cmd, args, env) =>
  spawn(cmd, args, { stdio: "inherit", shell: true, env: { ...process.env, ...env } })
    .on("exit", (code) => process.exit(code ?? 1));

if (mode === "cargo") {
  run("cargo", rest);
} else if (mode === "build") {
  run("tauri", ["build", ...rest]);            // a real bundle reads the real config
} else {
  run("tauri", ["dev", ...rest], { PATCHBAY_CONFIG: DEV_CONFIG });
}
