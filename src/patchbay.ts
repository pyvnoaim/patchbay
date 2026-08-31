import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { parse } from "smol-toml";

export type Jack = {
  host: string;
  user?: string;
  port?: number;
  key?: string;
  jump?: string;
  os?: string;
  url?: string;
  tags?: string[];
  desc?: string;
  forward?: string[];
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

const expand = (p: string) => (p.startsWith("~") ? homedir() + p.slice(1) : p);

/** [defaults] merges into every jack — that's the whole credential-inheritance feature. */
export function load(path = configPath()): Jacks {
  const raw = parse(readFileSync(path, "utf8")) as {
    defaults?: Partial<Jack>;
    jack?: Record<string, Jack>;
  };
  return Object.fromEntries(
    Object.entries(raw.jack ?? {}).map(([name, j]) => [name, { ...raw.defaults, ...j }]),
  );
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
  !filter || name.includes(filter) || (j.tags ?? []).some((t) => t.includes(filter));
