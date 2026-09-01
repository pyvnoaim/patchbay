import assert from "node:assert/strict";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { configPath, load, loadAll, resolve, sshArgs, type Jacks } from "../src/patchbay.ts";

const jacks: Jacks = {
  bastion: { host: "bastion.example", user: "jump", port: 2222 },
  web: { host: "10.0.0.4", user: "deploy", key: "~/.ssh/prod", jump: "bastion" },
  db: { host: "10.0.0.5", jump: "web", forward: ["5432:localhost:5432"] },
  loop: { host: "a", jump: "loop2" },
  loop2: { host: "b", jump: "loop" },
  raw: { host: "c", jump: "someone@elsewhere" },
};

test("plain jack is just user@host", () => {
  assert.deepEqual(sshArgs("bastion", jacks), ["-p", "2222", "jump@bastion.example"]);
});

test("jump chains dial the outermost bastion first, as ssh -J expects", () => {
  // db is reached via web, web via bastion — so from here the order is bastion, then web.
  assert.deepEqual(sshArgs("db", jacks), [
    "-J",
    "jump@bastion.example:2222,deploy@10.0.0.4",
    "-L",
    "5432:localhost:5432",
    "10.0.0.5",
  ]);
  assert.deepEqual(sshArgs("web", jacks).slice(0, 2), ["-J", "jump@bastion.example:2222"]);
});

test("~ in key expands, unknown jump passes through raw", () => {
  assert.match(sshArgs("web", jacks).join(" "), /-i \/.+\/\.ssh\/prod/);
  assert.deepEqual(sshArgs("raw", jacks), ["-J", "someone@elsewhere", "c"]);
});

test("jump loops throw instead of hanging", () => {
  assert.throws(() => sshArgs("loop", jacks), /loop/);
});

test("resolve: exact wins, unique substring works, ambiguity throws", () => {
  assert.equal(resolve("web", jacks), "web");
  assert.equal(resolve("bast", jacks), "bastion");
  assert.equal(resolve("loop", jacks), "loop"); // exact beats the loop2 substring hit
  assert.throws(() => resolve("loo", jacks), /matches 2/);
  assert.throws(() => resolve("nope", jacks), /no jack/);
});

test("config path: env overrides win, Windows lands in %APPDATA%", () => {
  const { PATCHBAY_CONFIG, XDG_CONFIG_HOME, APPDATA } = process.env;
  const platform = process.platform;
  const setPlatform = (v: string) => Object.defineProperty(process, "platform", { value: v });
  try {
    delete process.env.PATCHBAY_CONFIG;
    process.env.XDG_CONFIG_HOME = join("x", "cfg");
    assert.equal(configPath(), join("x", "cfg", "patchbay", "patchbay.toml"));

    delete process.env.XDG_CONFIG_HOME;
    process.env.APPDATA = join("C:", "Roaming");
    setPlatform("win32");
    assert.equal(configPath(), join("C:", "Roaming", "patchbay", "patchbay.toml"));

    setPlatform("linux");
    assert.match(configPath(), /\.config[\\/]patchbay[\\/]patchbay\.toml$/);

    process.env.PATCHBAY_CONFIG = "/tmp/override.toml";
    assert.equal(configPath(), "/tmp/override.toml");
  } finally {
    setPlatform(platform);
    Object.assign(process.env, { PATCHBAY_CONFIG, XDG_CONFIG_HOME, APPDATA });
    for (const [k, v] of Object.entries({ PATCHBAY_CONFIG, XDG_CONFIG_HOME, APPDATA }))
      if (v === undefined) delete process.env[k];
  }
});

test("[defaults] merge into jacks, jack wins", () => {
  const path = join(tmpdir(), `patchbay-${process.pid}.toml`);
  writeFileSync(path, `[defaults]\nuser = "root"\n\n[jack.a]\nhost = "h1"\n\n[jack.b]\nhost = "h2"\nuser = "me"\n`);
  const loaded = load(path);
  assert.equal(loaded.a!.user, "root");
  assert.equal(loaded.b!.user, "me");
});

test("`tags` still reads as `folders`, and `folders` wins when both are there", () => {
  const path = join(tmpdir(), `patchbay-folders-${process.pid}.toml`);
  writeFileSync(
    path,
    `[jack.old]\nhost = "h1"\ntags = ["prod/eu"]\n\n` +
      `[jack.new]\nhost = "h2"\nfolders = ["prod/us"]\ntags = ["stale"]\n`,
  );
  const loaded = load(path);
  assert.deepEqual(loaded.old!.folders, ["prod/eu"]);
  assert.deepEqual(loaded.new!.folders, ["prod/us"]);
  // Normalised away on load, so nothing downstream has to know the old name.
  assert.equal(loaded.old!.tags, undefined);
  assert.equal(loaded.new!.tags, undefined);
});

test("a jack's own `tags` beats `folders` inherited from [defaults]", () => {
  const path = join(tmpdir(), `patchbay-inherit-${process.pid}.toml`);
  writeFileSync(path, `[defaults]\nfolders = ["inherited"]\n\n[jack.a]\nhost = "h1"\ntags = ["mine"]\n`);
  // Mirrors the Rust test of the same name — the two disagreed here once.
  assert.deepEqual(load(path).a!.folders, ["mine"]);
});

test("every space loads, the main config wins a collision, and [defaults] stay put", () => {
  // Mirrors the Rust test of the same name.
  const dir = join(tmpdir(), `patchbay-spaces-${process.pid}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(join(dir, "spaces"), { recursive: true });
  const cfg = join(dir, "patchbay.toml");
  writeFileSync(cfg, `[jack.mine]\nhost = "h1"\n\n[jack.both]\nhost = "ours"\n`);
  writeFileSync(
    join(dir, "spaces", "acme.toml"),
    `[defaults]\nuser = "root"\n\n[jack.theirs]\nhost = "h2"\n\n[jack.both]\nhost = "theirs"\n`,
  );
  // Neither is a space: they sit beside one and end in something else.
  writeFileSync(join(dir, "spaces", "acme.toml.base"), `[jack.stale]\nhost = "old"\n`);
  writeFileSync(join(dir, "spaces", "acme.toml.bak"), `[jack.older]\nhost = "older"\n`);

  const all = loadAll(cfg);
  assert.deepEqual(Object.keys(all).sort(), ["both", "mine", "theirs"]);
  assert.equal(all.mine!.space, undefined);
  assert.equal(all.theirs!.space, "acme");
  assert.equal(all.both!.host, "ours");
  // A space's [defaults] are that space's, not everyone's.
  assert.equal(all.theirs!.user, "root");
  assert.equal(all.mine!.user, undefined);
});

test("no spaces directory is not an error", () => {
  const dir = join(tmpdir(), `patchbay-nospaces-${process.pid}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const cfg = join(dir, "patchbay.toml");
  writeFileSync(cfg, `[jack.a]\nhost = "h1"\n`);
  assert.deepEqual(Object.keys(loadAll(cfg)), ["a"]);
});
