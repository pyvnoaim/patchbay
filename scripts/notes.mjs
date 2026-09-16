// `node scripts/notes.mjs 0.1.0` - the CHANGELOG section for one version, which the
// release workflow hands to `gh release create --notes-file`.
import { readFileSync } from "node:fs";

const version = process.argv[2];
const log = readFileSync("CHANGELOG.md", "utf8");

// Everything under `## <version>` until the next `##` heading. `npm run bump` writes
// that heading, so a missing one means the changelog was never stamped.
const at = log.split(/^## /m).find((s) => s.startsWith(version));
if (!at) {
  console.error(`CHANGELOG.md has no section for ${version} - did you run npm run bump?`);
  process.exit(1);
}
const notes = at.split("\n").slice(1).join("\n").trim();
if (!notes) {
  console.error(`the ${version} section in CHANGELOG.md is empty`);
  process.exit(1);
}

console.log(notes);
