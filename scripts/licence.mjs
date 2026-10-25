// `npm run licence "Acme GmbH" 2027-10-20` - one team licence, printed. Paste the whole
// block into the reply; Settings ▸ Licence takes it as it is. The date is the last day it
// runs, a year from the order; licence.rs gives a week's grace after it.
//
// The payload is signed, so the company and the date cannot be edited afterwards. Both
// halves are joined by a `--` line and `licence.rs` splits on the same line: the bytes
// above it are what minisign covered, so nothing here may reformat them.
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [company, day] = process.argv.slice(2);
if (!company) {
  console.error('usage: npm run licence "Acme GmbH" [YYYY-MM-DD]');
  process.exit(1);
}

// A licence with no date never ends; every one we sell has one.
let until = null;
if (day) {
  // The end of that day, so a build made on it is still covered.
  const at = new Date(`${day}T23:59:59Z`);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day) || Number.isNaN(at.getTime())) {
    console.error(`"${day}" is not a YYYY-MM-DD date`);
    process.exit(1);
  }
  until = Math.floor(at.getTime() / 1000);
}

// TOML, because that is what the app parses it with. The quotes are escaped the way
// toml wants them, and a company name is the only thing here that came from outside.
const payload =
  `company = "${company.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"\n` +
  (until === null ? "" : `until = ${until}\n`);

const dir = mkdtempSync(join(tmpdir(), "patchbay-licence-"));
const file = join(dir, "licence.toml");
writeFileSync(file, payload);

const key =
  process.env.PATCHBAY_LICENCE_KEY ?? join(process.env.HOME ?? "", ".tauri/patchbay-licence.key");
try {
  execFileSync("npx", ["--no-install", "tauri", "signer", "sign", "-f", key, "-p", "", file], {
    stdio: ["ignore", "ignore", "inherit"],
  });
} catch {
  console.error(`could not sign with ${key} - is the private key there?`);
  process.exit(1);
}

// The CLI writes the signature beside the file, base64 of the minisign block. The app
// reads the block itself, so it is decoded here rather than adding a decoder to Rust.
const signature = Buffer.from(readFileSync(`${file}.sig`, "utf8").trim(), "base64").toString(
  "utf8",
);
process.stdout.write(`${payload}--\n${signature.trim()}\n`);
