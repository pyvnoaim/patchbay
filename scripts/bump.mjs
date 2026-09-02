// `npm run bump 0.1.0` - the version lives in four files and the release workflow
// refuses a tag that disagrees with tauri.conf.json.
//
// Targeted line rewrites rather than parse-and-serialize: package.json and the lock
// file would come back reformatted, and a version bump has no business touching
// anything else in the diff.
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("usage: npm run bump 0.1.0");
  process.exit(1);
}

// The lock's entry is the one that isn't a version *declaration*, so it is matched by
// the package it belongs to - patchbay-app is not the only crate in there with a
// version line, and every other one belongs to somebody else.
const files = [
  ["package.json", /("version":\s*")[^"]+(")/],
  ["src-tauri/tauri.conf.json", /("version":\s*")[^"]+(")/],
  ["src-tauri/Cargo.toml", /(^version = ")[^"]+(")/m],
  ["src-tauri/Cargo.lock", /(name = "patchbay-app"\nversion = ")[^"]+(")/],
];

for (const [file, pattern] of files) {
  const before = readFileSync(file, "utf8");
  if (!pattern.test(before)) {
    console.error(`no version to rewrite in ${file}`);
    process.exit(1);
  }
  writeFileSync(file, before.replace(pattern, `$1${version}$2`));
}

// The heading the release workflow looks for, so the notes are stamped by the same
// command that sets the version - remembering to do it by hand is how a release ends
// up shipping the previous one's notes.
// Anchored to its own line: the file explains this heading in its own prose, and a
// loose match stamps the sentence about the heading instead of the heading.
const heading = /^## Unreleased$/m;
const log = readFileSync("CHANGELOG.md", "utf8");
if (!heading.test(log)) {
  console.error("CHANGELOG.md has no `## Unreleased` section to release");
  process.exit(1);
}
const today = new Date().toISOString().slice(0, 10);
writeFileSync("CHANGELOG.md", log.replace(heading, `## ${version} - ${today}`));

console.log(`${version}\n\n  git commit -am "release ${version}" && git tag v${version} && git push --tags`);
