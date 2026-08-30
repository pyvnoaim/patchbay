// Copies the xterm UMD builds into ui/vendor/ so they load as plain <script> tags.
// The UI has no bundler and the CSP is `script-src 'self'`, so they have to be
// served from our own directory. Run `npm run vendor` after bumping either package.
import { copyFileSync, mkdirSync } from "node:fs";

const FILES = [
  ["@xterm/xterm/lib/xterm.js", "xterm.js"],
  ["@xterm/xterm/css/xterm.css", "xterm.css"],
  ["@xterm/addon-fit/lib/addon-fit.js", "addon-fit.js"],
];

mkdirSync("ui/vendor", { recursive: true });
for (const [from, to] of FILES) copyFileSync(`node_modules/${from}`, `ui/vendor/${to}`);
console.log(`vendored ${FILES.length} files into ui/vendor/`);
