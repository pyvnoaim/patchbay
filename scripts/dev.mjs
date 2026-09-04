// The one entry point for running patchbay locally.
//
//   node scripts/dev.mjs          the app window
//   node scripts/dev.mjs build    a release bundle
//   node scripts/dev.mjs cargo …  cargo, with PATH sorted out
//
// npm scripts can't do two things portably: point at the dev config (an env-var prefix doesn't
// work in cmd.exe) and find cargo (a terminal opened before rustup ran has no cargo on PATH).
import { spawn } from "node:child_process";
import { copyFileSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

const [mode = "dev", ...rest] = process.argv.slice(2);

// `tauri dev` runs the binary with src-tauri/ as its cwd, so this has to be absolute.
const DEV_CONFIG = resolve("dev/patchbay.toml");

// The dev config isn't tracked, because it fills up with real addresses. Seed it from the example.
if (!existsSync(DEV_CONFIG)) copyFileSync(resolve("dev/patchbay.example.toml"), DEV_CONFIG);

const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) process.env.PATH = `${cargoBin}${delimiter}${process.env.PATH}`;

// The updater signs every build or prints an error that reads like a failure. The key never lives
// in the repo; TAURI_SIGNING_PRIVATE_KEY takes a path or the key itself, and CI passes the key.
// It is password-protected, so a local build also wants TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
const signingKey = join(homedir(), ".tauri", "patchbay.key");
if (existsSync(signingKey) && !process.env.TAURI_SIGNING_PRIVATE_KEY) {
  process.env.TAURI_SIGNING_PRIVATE_KEY = signingKey;
}

const run = (cmd, args, env) =>
  spawn(cmd, args, { stdio: "inherit", shell: true, env: { ...process.env, ...env } }).on(
    "exit",
    (code) => process.exit(code ?? 1),
  );

if (mode === "cargo") {
  run("cargo", rest);
} else if (mode === "build") {
  run("tauri", ["build", ...rest]); // a real bundle reads the real config
} else {
  run("tauri", ["dev", ...rest], { PATCHBAY_CONFIG: DEV_CONFIG });
}
