// Turning an existing ssh config into jacks. Prints, never writes: config.rs is the
// only thing that edits a patchbay.toml, and an importer that respects that is one
// you can run twice without fear.
import { stringify } from "smol-toml";

export type Imported = {
  name: string;
  host: string;
  user?: string;
  port?: number;
  key?: string;
  jump?: string;
};

/** Everything else ssh already applies for us — we exec it, so importing its
 *  defaults would only duplicate them into a second file that can go stale. */
const WANTED = new Set(["hostname", "user", "port", "identityfile", "proxyjump"]);

const unquote = (v: string) =>
  v.length > 1 && v.startsWith('"') && v.endsWith('"') ? v.slice(1, -1) : v;

/** A pattern, not a host: ssh matches these, patchbay can't list them. */
const isPattern = (h: string) => h.startsWith("!") || h.includes("*") || h.includes("?");

/** Warnings come back rather than going to a callback: the CLI prints them, the app
 *  shows them, and `import.rs` can mirror the signature exactly. */
export function fromSshConfig(src: string): { hosts: Imported[]; warnings: string[] } {
  const out: Imported[] = [];
  const warnings: string[] = [];
  const byAlias = new Map<string, string>();
  const taken = new Set<string>();
  let aliases: string[] = [];
  let block: Record<string, string> = {};
  let included = false;

  const flush = () => {
    let jump = block.proxyjump === "none" ? undefined : block.proxyjump;
    if (jump?.includes(",")) {
      // `jump` is the one hop before the target; a chain is built by pointing jacks
      // at each other, which we can't synthesise from a list of raw specs.
      const hops = jump.split(",").map((h) => h.trim());
      jump = hops.at(-1);
      warnings.push(`${aliases.join(", ")}: ProxyJump has ${hops.length} hops — kept "${jump}", dropped the rest`);
    }
    for (const alias of aliases) {
      // A dot is refused in a jack name, and two aliases can flatten onto one.
      const base = alias.replace(/\./g, "-");
      let name = base;
      for (let n = 2; taken.has(name); n++) name = `${base}-${n}`;
      taken.add(name);
      byAlias.set(alias, name);

      const port = Number(block.port);
      out.push({
        name,
        host: block.hostname ?? alias,
        user: block.user,
        port: Number.isInteger(port) && port > 0 ? port : undefined,
        key: block.identityfile,
        jump,
      });
    }
    aliases = [];
    block = {};
  };

  for (const raw of src.split(/\r?\n/)) {
    const line = raw.trim();
    // ssh only treats `#` as a comment at the start of a line — a trailing one is
    // part of the value, so stripping it would corrupt a password-shaped path.
    if (!line || line.startsWith("#")) continue;

    const [word, ...rest] = line.replace(/=/, " ").split(/\s+/);
    const key = word.toLowerCase();
    const value = unquote(rest.join(" ").trim());

    if (key === "host") {
      flush();
      aliases = rest.filter((h) => !isPattern(h));
      continue;
    }
    // A Match block's settings hang off conditions, not a host, so nothing in it
    // belongs to a jack.
    if (key === "match") {
      flush();
      continue;
    }
    if (key === "include") {
      included = true;
      continue;
    }
    // First one wins, the way ssh reads them.
    if (WANTED.has(key) && value && !(key in block)) block[key] = value;
  }
  flush();

  if (included) warnings.push("Include lines were not followed — run the importer on those files too");

  // A ProxyJump naming another Host has to point at that jack's sanitised name;
  // anything else is a raw spec, which patchbay passes through to ssh untouched.
  for (const j of out) if (j.jump && byAlias.has(j.jump)) j.jump = byAlias.get(j.jump);

  return { hosts: out, warnings };
}

export function toToml(jacks: Imported[]): string {
  const jack = Object.fromEntries(
    jacks.map(({ name, ...rest }) => [
      name,
      Object.fromEntries(Object.entries(rest).filter(([, v]) => v !== undefined)),
    ]),
  );
  return `# ${jacks.length} host${jacks.length === 1 ? "" : "s"} from your ssh config.\n` +
    `# Check it, then paste it into your patchbay.toml.\n\n${stringify({ jack })}\n`;
}
