// Launches the Tauri CLI with whatever env node --env-file already loaded, so
// `npm run dev` opens the app against patchbay.dev.toml instead of your real config.
// (An env-var prefix in an npm script doesn't work on Windows; this does.)
import { spawn } from "node:child_process";
import { resolve } from "node:path";

// `tauri dev` runs the binary with src-tauri/ as its cwd, so the relative path in
// dev.env has to be absolute before it's handed over.
if (process.env.PATCHBAY_CONFIG) process.env.PATCHBAY_CONFIG = resolve(process.env.PATCHBAY_CONFIG);

spawn("tauri", process.argv.slice(2), { stdio: "inherit", shell: true }).on("exit", (code) =>
  process.exit(code ?? 1),
);
