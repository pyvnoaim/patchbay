import { existsSync, readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { parse } from "smol-toml";

export type Jack = {
  host: string;
  user?: string;
  port?: number;
  key?: string;
  jump?: string;
  os?: string;
  url?: string;
  rdp?: number;
  vnc?: number;
  ssh?: boolean;
  primary?: string;
  folders?: string[];
  /** ponytail: the old name for `folders`. Read so existing files still work,
      never written; drop it once nobody has one. */
  tags?: string[];
  desc?: string;
  forward?: string[];
  /** Which space this came from — the file it was in, not a field anyone writes. */
  space?: string;
};

export type Jacks = Record<string, Jack>;

/** %APPDATA% on Windows, $XDG_CONFIG_HOME or ~/.config everywhere else. */
const configHome = (): string => {
  if (process.env.XDG_CONFIG_HOME) return process.env.XDG_CONFIG_HOME;
  if (process.platform === "win32" && process.env.APPDATA) return process.env.APPDATA;
  return join(homedir(), ".config");
};

export const configPath = (): string =>
  process.env.PATCHBAY_CONFIG ?? join(configHome(), "patchbay", "patchbay.toml");

/** Beside the config: one file per extra space. A space *is* a config, whole. */
export const spacesDir = (cfg = configPath()): string => join(dirname(cfg), "spaces");

/** Where a named space lives. No name is the main config — that one is your own list. */
export const spacePath = (space?: string, cfg = configPath()): string =>
  space ? join(spacesDir(cfg), `${space}.toml`) : cfg;

/**
 * Every space that exists: the main config first, then `spaces/*.toml` sorted.
 * The `.toml` test is load-bearing — a space's `.toml.base` and `.toml.bak` sit in
 * the same directory and are not spaces.
 */
export function spacePaths(cfg = configPath()): [string | undefined, string][] {
  const out: [string | undefined, string][] = existsSync(cfg) ? [[undefined, cfg]] : [];
  let names: string[];
  try {
    names = readdirSync(spacesDir(cfg));
  } catch {
    return out;
  }
  for (const f of names.filter((n) => n.endsWith(".toml")).sort()) {
    out.push([f.slice(0, -".toml".length), join(spacesDir(cfg), f)]);
  }
  return out;
}

const expand = (p: string) => (p.startsWith("~") ? homedir() + p.slice(1) : p);

/** [defaults] merges into every jack — that's the whole credential-inheritance feature. */
export function load(path = configPath()): Jacks {
  const raw = parse(readFileSync(path, "utf8")) as {
    defaults?: Partial<Jack>;
    jack?: Record<string, Jack>;
  };
  // Normalised before merging, not after: spreading first lets a `folders` in
  // [defaults] hide a jack's own legacy `tags`, and the jack has to win.
  const folders = (j: Partial<Jack>): Partial<Jack> => {
    const out = { ...j };
    if (!out.folders && out.tags) out.folders = out.tags;
    delete out.tags;
    return out;
  };
  const defaults = folders(raw.defaults ?? {});
  return Object.fromEntries(
    Object.entries(raw.jack ?? {}).map(([name, j]) => [name, { ...defaults, ...folders(j) }]),
  );
}

/**
 * Every space's jacks in one map. Each file resolves on its own, so `[defaults]` in
 * a space applies to that space's jacks and nobody else's.
 *
 * ponytail: a name in two spaces resolves to the first one — the main config, then
 * spaces alphabetically. Qualify as "acme:web" if two spaces ever collide in practice.
 */
export function loadAll(cfg = configPath()): Jacks {
  const out: Jacks = {};
  for (const [space, path] of spacePaths(cfg)) {
    for (const [name, j] of Object.entries(load(path))) {
      if (!(name in out)) out[name] = space ? { ...j, space } : j;
    }
  }
  return out;
}

const spec = (j: Jack) => `${j.user ? j.user + "@" : ""}${j.host}`;

/**
 * The jump chain, ordered the way `ssh -J` wants it: leftmost is the first hop
 * from here. Walking `jump` goes outward from the target, so the walk is reversed —
 * `db → web → bastion` has to dial bastion first, not web.
 */
export function hops(name: string, jacks: Jacks): string[] {
  const j = jacks[name];
  if (!j) throw new Error(`no jack named "${name}"`);

  const out: string[] = [];
  const seen = new Set([name]);
  for (let hop: string | undefined = j.jump; hop; ) {
    if (seen.has(hop)) throw new Error(`jump loop through "${hop}"`);
    seen.add(hop);
    const via: Jack | undefined = jacks[hop];
    if (!via) {
      out.push(hop); // not a jack name, pass through as a raw ssh spec
      break;
    }
    out.push(via.port ? `${spec(via)}:${via.port}` : spec(via));
    hop = via.jump;
  }
  return out.reverse();
}

export function sshArgs(name: string, jacks: Jacks): string[] {
  const j = jacks[name]!;
  const hopList = hops(name, jacks);

  const args: string[] = [];
  if (hopList.length) args.push("-J", hopList.join(","));
  if (j.port) args.push("-p", String(j.port));
  if (j.key) args.push("-i", expand(j.key));
  for (const f of j.forward ?? []) args.push("-L", f);
  args.push(spec(j));
  return args;
}

/** Exact name wins; otherwise substring match, but only if it's unambiguous. */
export function resolve(query: string, jacks: Jacks): string {
  if (jacks[query]) return query;
  const hits = Object.keys(jacks).filter((n) => n.includes(query));
  if (hits.length === 1) return hits[0]!;
  if (hits.length === 0) throw new Error(`no jack matching "${query}"`);
  throw new Error(`"${query}" matches ${hits.length} jacks: ${hits.join(", ")}`);
}

export const matches = (j: Jack, name: string, filter?: string) =>
  !filter || name.includes(filter) || (j.folders ?? []).some((f) => f.includes(filter));
