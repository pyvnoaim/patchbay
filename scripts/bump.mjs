// `npm run bump [0.1.0]`: the version lives in four files, and CI refuses a `release x.y.z`
// commit that disagrees with tauri.conf.json. Targeted line rewrites, so nothing else is reformatted.
// No version is the next patch, because every push is a release.
import { readFileSync, writeFileSync } from "node:fs";

const current = JSON.parse(readFileSync("package.json", "utf8")).version;
const version = process.argv[2] ?? current.replace(/\d+$/, (n) => +n + 1);
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error("usage: npm run bump [0.1.0]");
  process.exit(1);
}

// Cargo.lock has a version line per crate, so the entry is matched by its name.
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

// The heading the release workflow looks for, stamped by the same command that sets the version.
// Anchored to its own line: the changelog's prose mentions this heading too.
const heading = /^## Unreleased$/m;
const log = readFileSync("CHANGELOG.md", "utf8");
if (!heading.test(log)) {
  console.error("CHANGELOG.md has no `## Unreleased` section to release");
  process.exit(1);
}
// The local date: `toISOString` is UTC, so a release cut after midnight anywhere east
// of it was dated yesterday.
const now = new Date();
const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(
  now.getDate(),
).padStart(2, "0")}`;
writeFileSync("CHANGELOG.md", log.replace(heading, `## ${version} - ${today}`));

console.log(`${version}\n\n  git commit -am "release ${version}" && git push origin main`);
