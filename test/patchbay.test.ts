import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { load, resolve, sshArgs, type Jacks } from "../src/patchbay.ts";

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

test("jump chains walk to the end, in order", () => {
  assert.deepEqual(sshArgs("db", jacks), [
    "-J",
    "deploy@10.0.0.4,jump@bastion.example:2222",
    "-L",
    "5432:localhost:5432",
    "10.0.0.5",
  ]);
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

test("[defaults] merge into jacks, jack wins", () => {
  const path = join(tmpdir(), `patchbay-${process.pid}.toml`);
  writeFileSync(path, `[defaults]\nuser = "root"\n\n[jack.a]\nhost = "h1"\n\n[jack.b]\nhost = "h2"\nuser = "me"\n`);
  const loaded = load(path);
  assert.equal(loaded.a!.user, "root");
  assert.equal(loaded.b!.user, "me");
});
