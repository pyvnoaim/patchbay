// The one entry point for running patchbay locally.
//
//   node scripts/dev.mjs          the app window
//   node scripts/dev.mjs build    a release bundle
//   node scripts/dev.mjs cli ls   the CLI
//   node scripts/dev.mjs cargo …  cargo, with PATH sorted out
//
// It exists because two things need fixing before either front end starts, and
// npm scripts can't do them portably: pointing at the sample config (an env-var
// prefix doesn't work in cmd.exe) and finding cargo (rustup edits your shell
// profile, so a terminal opened before you installed it has no cargo on PATH).
import { spawn, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
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

const run = (cmd, args, env) =>
  spawn(cmd, args, { stdio: "inherit", shell: true, env: { ...process.env, ...env } })
    .on("exit", (code) => process.exit(code ?? 1));

// The window ships the CLI beside it, so "Install the bay command" has something to link:
// a sibling of the app binary, which target/ gives us in dev and the bundler's externalBin
// gives us in a build. The sidecar is declared in bundle.conf.json rather than
// tauri.conf.json because tauri-build checks an externalBin exists on *every* cargo run,
// `cargo test` included, and bundling is the only step that needs one.
function stageBay(release) {
  const args = ["build", "--manifest-path", "src-tauri/Cargo.toml", "--bin", "bay"];
  if (release) args.push("--release");
  const built = spawnSync("cargo", args, { stdio: "inherit", shell: true });
  if (built.status) process.exit(built.status);
  if (!release) return;   // dev needs no copy: the sibling in target/ is what's looked for

  const host = spawnSync("rustc", ["-vV"], { encoding: "utf8", shell: true }).stdout
    ?.match(/^host: (.+)$/m)?.[1];
  if (!host) { console.error("no rustc on PATH"); process.exit(1); }
  const ext = process.platform === "win32" ? ".exe" : "";
  mkdirSync(resolve("src-tauri/binaries"), { recursive: true });
  copyFileSync(resolve(`src-tauri/target/release/bay${ext}`),
               resolve(`src-tauri/binaries/bay-${host}${ext}`));
}

if (mode === "cli") {
  run("node", ["--experimental-strip-types", "--disable-warning=ExperimentalWarning", "src/cli.ts", ...rest],
      { PATCHBAY_CONFIG: DEV_CONFIG });
} else if (mode === "cargo") {
  run("cargo", rest);
} else if (mode === "build") {
  stageBay(true);
  run("tauri", ["build", "--config", "src-tauri/bundle.conf.json", ...rest]);   // a real bundle reads the real config
} else {
  stageBay(false);
  run("tauri", ["dev", ...rest], { PATCHBAY_CONFIG: DEV_CONFIG });
}
