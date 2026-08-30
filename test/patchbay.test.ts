import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { configPath, load, resolve, sshArgs, type Jacks } from "../src/patchbay.ts";

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
