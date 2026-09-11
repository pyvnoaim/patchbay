// The one entry point for running patchbay locally.
//
//   npm run dev      the app window
//   npm run build    a release bundle, and on macOS opens the .dmg it made
//   npm test         cargo, with PATH sorted out
//
// Always through npm: `tauri` is resolved off node_modules/.bin, which only npm puts on PATH.
//
// npm scripts can't do two things portably: point at the dev config (an env-var prefix doesn't
// work in cmd.exe) and find cargo (a terminal opened before rustup ran has no cargo on PATH).
import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, readdirSync, rmSync, statSync } from "node:fs";
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

const run = (cmd, args, env, then) =>
  spawn(cmd, args, { stdio: "inherit", shell: true, env: { ...process.env, ...env } }).on(
    "exit",
    (code) => {
      if (code === 0) then?.();
      process.exit(code ?? 1);
    },
  );

// The disk image lands four directories down, where nobody finds it. Only one written by this
// build: a `--target` or `--bundles app` run leaves an older image in this folder behind.
function openDmg(since) {
  if (process.platform !== "darwin") return;
  const dir = resolve("src-tauri/target/release/bundle/dmg");
  if (!existsSync(dir)) return;
  const dmg = readdirSync(dir)
    .map((f) => join(dir, f))
    .find((f) => f.endsWith(".dmg") && statSync(f).mtimeMs >= since);
  if (!dmg) return;
  dropBuiltApp();
  spawnSync("open", [dmg]);
}

// Spotlight registers the bundle the image was made from, so Launchpad and Open With list a second
// patchbay beside the installed one. Once it is in the image nothing reads it; the updater's
// .tar.gz beside it stays. Unregistered first, or Launchpad keeps a dead icon until the next login.
const LSREGISTER =
  "/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister";
function dropBuiltApp() {
  const app = resolve("src-tauri/target/release/bundle/macos/patchbay.app");
  if (!existsSync(app)) return;
  spawnSync(LSREGISTER, ["-u", app]);
  rmSync(app, { recursive: true, force: true });
}

if (mode === "cargo") {
  run("cargo", rest);
} else if (mode === "build") {
  const started = Date.now();
  run("tauri", ["build", ...rest], {}, () => openDmg(started)); // a real bundle reads the real config
} else {
  run("tauri", ["dev", ...rest], { PATCHBAY_CONFIG: DEV_CONFIG });
}
