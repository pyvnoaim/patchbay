#!/usr/bin/env -S node --experimental-strip-types --disable-warning=ExperimentalWarning
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { styleText } from "node:util";
import { configPath, load, matches, resolve, sshArgs, type Jacks } from "./patchbay.ts";

const TEMPLATE = `# patchbay — every host, one jack away
# Anything here is inherited by every jack below.
[defaults]
user = "root"

[jack.example]
host = "192.0.2.10"
tags = ["demo"]
desc = "delete me"

# [jack.prod-web]
# host = "10.0.0.4"
# user = "deploy"
# key  = "~/.ssh/prod"
# jump = "bastion"              # another jack name, or a raw user@host
# forward = ["8080:localhost:80"]
`;

const c = process.stdout.isTTY ? styleText : (_f: unknown, s: string) => s;
const die = (msg: string): never => {
  console.error(c("red", msg));
  process.exit(1);
};

function list(jacks: Jacks, filter?: string) {
  const rows = Object.entries(jacks).filter(([name, j]) => matches(j, name, filter));
  if (!rows.length) return die(filter ? `nothing matches "${filter}"` : "no jacks configured");
  const w = Math.max(...rows.map(([n]) => n.length));
  for (const [name, j] of rows) {
    const tags = j.tags?.length ? c("dim", ` [${j.tags.join(" ")}]`) : "";
    console.log(`${c("yellow", name.padEnd(w))}  ${c("dim", j.host)}${tags}`);
  }
}

/** fzf if it's there, plain list if it isn't. ponytail: no bundled picker, add one when fzf annoys you. */
function pick(names: string[]): string | undefined {
  const r = spawnSync("fzf", ["--height=40%", "--reverse", "--prompt=jack> "], {
    input: names.join("\n"),
    encoding: "utf8",
    stdio: ["pipe", "pipe", "inherit"],
  });
  return r.status === 0 ? r.stdout.trim() || undefined : undefined;
}

function edit() {
  const path = configPath();
  if (!existsSync(path)) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, TEMPLATE);
    console.error(c("dim", `created ${path}`));
  }
  const editor = process.env.VISUAL ?? process.env.EDITOR ?? "vi";
  process.exit(spawnSync(editor, [path], { stdio: "inherit" }).status ?? 0);
}

const [cmd, ...rest] = process.argv.slice(2);

if (cmd === "-h" || cmd === "--help") {
  console.log(`bay              pick a jack (fzf) or list them
bay <name>       connect — substring is enough
bay <name> -- <cmd>   run a command instead of a shell
bay ls [filter]  list jacks, filtered by name or tag
bay edit         open ${configPath()}`);
  process.exit(0);
}
if (cmd === "edit") edit();

const path = configPath();
if (!existsSync(path)) die(`no config at ${path} — run \`bay edit\` to start one`);

let jacks: Jacks;
try {
  jacks = load(path);
} catch (e) {
  die(`${path}: ${(e as Error).message}`);
}

if (cmd === "ls") {
  list(jacks!, rest[0]);
  process.exit(0);
}

const target = cmd ?? pick(Object.keys(jacks!));
if (!target) {
  list(jacks!);
  process.exit(0);
}

try {
  const dash = rest.indexOf("--");
  const args = sshArgs(resolve(target, jacks!), jacks!);
  if (dash !== -1) args.push(...rest.slice(dash + 1));
  process.exit(spawnSync("ssh", args, { stdio: "inherit" }).status ?? 1);
} catch (e) {
  die((e as Error).message);
}
