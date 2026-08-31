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
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

const [mode = "dev", ...rest] = process.argv.slice(2);

// `tauri dev` runs the binary with src-tauri/ as its cwd, so this has to be absolute.
const DEV_CONFIG = resolve("dev/patchbay.toml");

const cargoBin = join(homedir(), ".cargo", "bin");
if (existsSync(cargoBin)) process.env.PATH = `${cargoBin}${delimiter}${process.env.PATH}`;

const run = (cmd, args, env) =>
  spawn(cmd, args, { stdio: "inherit", shell: true, env: { ...process.env, ...env } })
    .on("exit", (code) => process.exit(code ?? 1));

if (mode === "cli") {
  run("node", ["--experimental-strip-types", "--disable-warning=ExperimentalWarning", "src/cli.ts", ...rest],
      { PATCHBAY_CONFIG: DEV_CONFIG });
} else if (mode === "cargo") {
  run("cargo", rest);
} else if (mode === "build") {
  run("tauri", ["build", ...rest]);            // a real bundle reads the real config
} else {
  run("tauri", ["dev", ...rest], { PATCHBAY_CONFIG: DEV_CONFIG });
}
