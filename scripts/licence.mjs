// `npm run licence "Acme GmbH" 2027-09-07` - one team licence, printed. Paste the whole
// block into the reply; Settings ▸ Team takes it as it is.
//
// The payload is signed, so the company and the expiry cannot be edited afterwards. Both
// halves are joined by a `--` line and `licence.rs` splits on the same line: the bytes
// above it are what minisign covered, so nothing here may reformat them.
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [company, until] = process.argv.slice(2);
if (!company) {
  console.error('usage: npm run licence "Acme GmbH" [YYYY-MM-DD]');
  process.exit(1);
}

// A licence with no date never lapses; every one we sell has one.
let expires = null;
if (until) {
  const at = new Date(`${until}T00:00:00Z`);
  if (Number.isNaN(at.getTime())) {
    console.error(`"${until}" is not a YYYY-MM-DD date`);
    process.exit(1);
  }
  expires = Math.floor(at.getTime() / 1000);
}

// TOML, because that is what the app parses it with. The quotes are escaped the way
// toml wants them, and a company name is the only thing here that came from outside.
const payload =
  `company = "${company.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"\n` +
  (expires === null ? "" : `expires = ${expires}\n`);

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
