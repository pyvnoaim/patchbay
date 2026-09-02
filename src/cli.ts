#!/usr/bin/env -S node --experimental-strip-types --disable-warning=ExperimentalWarning
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { styleText } from "node:util";
import { fromSshConfig, toToml } from "./import.ts";
import {
  configPath, loadAll, matches, primary, resolve, spacePath, sshArgs,
  type Jack, type Jacks,
} from "./patchbay.ts";

const TEMPLATE = `# patchbay - every host, one jack away
# Anything here is inherited by every jack below.
[defaults]
user = "root"

[jack.example]
host = "192.0.2.10"
folders = ["demo"]
desc = "delete me"

# [jack.prod-web]
# host = "10.0.0.4"
# user = "deploy"
# key  = "~/.ssh/prod"
# jump = "bastion"              # another jack name, or a raw user@host
# forward = ["8080:localhost:80"]
`;

const win = process.platform === "win32";
const c = process.stdout.isTTY ? styleText : (_f: unknown, s: string) => s;
const die = (msg: string): never => {
  console.error(c("red", msg));
  process.exit(1);
};

/** Name, host, folders and - once you have more than your own list - the space. */
const row = (name: string, j: Jack, w: number) =>
  `${name.padEnd(w)}  ${j.host}` +
  (j.folders?.length ? ` [${j.folders.join(" ")}]` : "") +
  (j.space ? ` @${j.space}` : "");

function select(jacks: Jacks, filter?: string, space?: string) {
  let rows = Object.entries(jacks).filter(([name, j]) => matches(j, name, filter));
  if (space !== undefined) rows = rows.filter(([, j]) => (j.space ?? "") === space);
  if (rows.length) return rows;
  if (filter) return die(`nothing matches "${filter}"`);
  return die(space ? `nothing in space "${space}"` : "no jacks configured");
}

function list(jacks: Jacks, filter?: string, opts: { names?: boolean; space?: string } = {}) {
  const rows = select(jacks, filter, opts.space);
  // One name per line, for the completion snippets and anything else piping this.
  if (opts.names) return void rows.forEach(([name]) => console.log(name));
  const w = Math.max(...rows.map(([n]) => n.length));
  for (const [name, j] of rows) {
    const folders = j.folders?.length ? c("dim", ` [${j.folders.join(" ")}]`) : "";
    const space = j.space ? c("dim", ` @${j.space}`) : "";
    console.log(`${c("yellow", name.padEnd(w))}  ${c("dim", j.host)}${folders}${space}`);
  }
}

/** fzf if it's there, plain list if it isn't. ponytail: no bundled picker, add one when fzf annoys you. */
function pick(jacks: Jacks): string | undefined {
  const rows = Object.entries(jacks);
  const w = Math.max(...rows.map(([n]) => n.length));
  // The columns are padded then joined by two spaces, so a name is field 1 - which is
  // what the preview and the returned line are read back as.
  // $BAY rather than a path we quote ourselves: fzf runs the preview through a shell.
  const r = spawnSync("fzf", [
    "--height=40%", "--reverse", "--prompt=jack> ",
    "--delimiter", "  ",
    "--preview", '"$BAY" {1} -n', "--preview-window", "down,3",
  ], {
    input: rows.map(([name, j]) => row(name, j, w)).join("\n"),
    encoding: "utf8",
    stdio: ["pipe", "pipe", "inherit"],
    env: { ...process.env, BAY: process.argv[1] },
  });
  return r.status === 0 ? r.stdout.trim().split(/\s{2,}/)[0] || undefined : undefined;
}

function edit(space?: string) {
  const path = spacePath(space);
  if (!existsSync(path)) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, TEMPLATE);
    console.error(c("dim", `created ${path}`));
  }
  const editor = process.env.VISUAL ?? process.env.EDITOR ?? (win ? "notepad" : "vi");
  // Windows editors are usually .cmd shims (code, subl), which spawn refuses without a shell.
  const r = win
    ? spawnSync(editor, [`"${path}"`], { stdio: "inherit", shell: true })
    : spawnSync(editor, [path], { stdio: "inherit" });
  process.exit(r.status ?? 0);
}

/** Names come from `bay ls --names`, so a snippet never goes stale with the config. */
const COMPLETIONS: Record<string, string> = {
  zsh: [
    '# eval "$(bay completion zsh)" in ~/.zshrc',
    "_bay() { compadd -- ${(f)\"$(bay ls --names)\"} }",
    "compdef _bay bay",
  ].join("\n"),
  bash: [
    '# eval "$(bay completion bash)" in ~/.bashrc',
    '_bay() { COMPREPLY=($(compgen -W "$(bay ls --names)" -- "$2")); }',
    "complete -F _bay bay",
  ].join("\n"),
  fish: [
    "# bay completion fish > ~/.config/fish/completions/bay.fish",
    'complete -c bay -f -a "(bay ls --names)"',
  ].join("\n"),
};

const [cmd, ...rest] = process.argv.slice(2);

if (cmd === "-h" || cmd === "--help") {
  console.log(`bay              pick a jack (fzf) or list them
bay <name>       connect - substring is enough, and any other flag goes to ssh
bay <name> -n    print the ssh command instead of running it
bay <name> -- <cmd>   run a command instead of a shell
bay ls [filter]  list jacks, filtered by name or folder
                 --names for names alone, --space <space> for one space
bay edit [space] open ${configPath()}, or spaces/<space>.toml
bay import [file]  print TOML for the hosts in your ssh config
bay completion <shell>   a snippet for zsh, bash or fish
bay --version`);
  process.exit(0);
}
if (cmd === "--version") {
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  console.log(`patchbay ${pkg.version}`);
  process.exit(0);
}
if (cmd === "completion") {
  const snippet = COMPLETIONS[rest[0] ?? ""];
  if (!snippet) die(`no completion for "${rest[0] ?? ""}" - zsh, bash or fish`);
  console.log(snippet);
  process.exit(0);
}
if (cmd === "edit") edit(rest[0]);

// Stdout, not the config file: config.rs is the only thing that edits a patchbay.toml,
// and printing means you read it before you keep it.
if (cmd === "import") {
  const file = rest[0] ?? join(homedir(), ".ssh", "config");
  if (!existsSync(file)) die(`no ssh config at ${file}`);
  const { hosts, warnings } = fromSshConfig(readFileSync(file, "utf8"));
  for (const w of warnings) console.error(c("yellow", w));
  if (!hosts.length) die(`no hosts in ${file}`);
  process.stdout.write(toToml(hosts));
  process.exit(0);
}

const path = configPath();
if (!existsSync(path)) die(`no config at ${path} - run \`bay edit\` to start one`);

let jacks: Jacks;
try {
  jacks = loadAll(path);
} catch (e) {
  die(`${path}: ${(e as Error).message}`);
}

if (cmd === "ls") {
  const si = rest.indexOf("--space");
  const space = si === -1 ? undefined : rest[si + 1];
  if (si !== -1 && space === undefined) die("--space needs a space name");
  const arg = si === -1 ? -1 : si + 1;   // --space's own argument isn't the filter
  const filter = rest.find((a, i) => !a.startsWith("--") && i !== arg);
  list(jacks!, filter, { names: rest.includes("--names"), space });
  process.exit(0);
}

const target = cmd ?? pick(jacks!);
if (!target) {
  list(jacks!);
  process.exit(0);
}

try {
  const dash = rest.indexOf("--");
  const name = resolve(target, jacks!);
  const j = jacks![name]!;
  // `ssh = false` is the device saying so; `primary` only names the default action, so a
  // box with a web ui *and* ssh still connects. Without this it built an ssh command for
  // an appliance that never listened on 22.
  if (j.ssh === false) {
    const how = primary(j);
    const what = how === "web" ? ` - it's a web ui at ${j.url}`
      : how === "rdp" ? ` - it's remote desktop on port ${j.rdp}`
        : how === "vnc" ? ` - it's vnc on port ${j.vnc}` : "";
    die(`"${name}" isn't reached by ssh${what}`);
  }

  const args = sshArgs(name, jacks!);
  const flags = dash === -1 ? rest : rest.slice(0, dash);
  const dry = flags.includes("-n") || flags.includes("--dry-run");
  // Everything else is ssh's, and ssh wants its options before the target - which
  // sshArgs puts last. Dropping them silently is how `bay web -v` stopped being verbose.
  args.splice(-1, 0, ...flags.filter((f) => f !== "-n" && f !== "--dry-run"));
  if (dash !== -1) args.push(...rest.slice(dash + 1));

  if (dry) {
    console.log(["ssh", ...args].join(" "));
    process.exit(0);
  }

  const r = spawnSync("ssh", args, { stdio: "inherit" });
  if ((r.error as NodeJS.ErrnoException | undefined)?.code === "ENOENT")
    die(win ? "no ssh on PATH - enable the OpenSSH Client feature in Windows Settings" : "no ssh on PATH");
  process.exit(r.status ?? 1);
} catch (e) {
  die((e as Error).message);
}
