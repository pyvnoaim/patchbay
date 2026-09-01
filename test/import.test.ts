import { test } from "node:test";
import assert from "node:assert/strict";
import { fromSshConfig, toToml } from "../src/import.ts";

const CONFIG = `
# my hosts
Host *
  ServerAliveInterval 30

Host bastion.example
  HostName 203.0.113.9
  User ops
  Port 2222
  IdentityFile ~/.ssh/ops

Host web prod-web
  HostName 10.0.0.4
  ProxyJump bastion.example

Host db
  HostName db.internal
  ProxyJump bastion.example,10.0.0.4

Host bare

Match host *.internal
  User root
`;

test("hosts become jacks, patterns and Match blocks do not", () => {
  const j = fromSshConfig(CONFIG);
  assert.deepEqual(j.map((x) => x.name), ["bastion-example", "web", "prod-web", "db", "bare"]);

  assert.deepEqual(j[0], {
    name: "bastion-example",
    host: "203.0.113.9",
    user: "ops",
    port: 2222,
    key: "~/.ssh/ops",
    jump: undefined,
  });

  // Two aliases on one Host line share the block, and a missing HostName means the
  // alias was already the hostname.
  assert.equal(j[1].host, "10.0.0.4");
  assert.equal(j[2].host, "10.0.0.4");
  assert.equal(j[4].host, "bare");

  // `Match` settings hang off a condition, not a host — nothing there is a jack, and
  // it must not leak into the block before it.
  assert.equal(j[4].user, undefined);
});

test("a jump pointing at another Host follows it to the renamed jack", () => {
  const j = fromSshConfig(CONFIG);
  assert.equal(j[1].jump, "bastion-example", "the dot is gone from the name it points at");
});

test("a multi-hop ProxyJump keeps the hop nearest the target and says so", () => {
  const warnings: string[] = [];
  const j = fromSshConfig(CONFIG, (m) => warnings.push(m));
  assert.equal(j[3].jump, "10.0.0.4", "the last -J entry is the one before the target");
  assert.ok(warnings.some((w) => w.includes("dropped the rest")), warnings.join("\n"));
});

test("colliding names get a suffix rather than overwriting each other", () => {
  const j = fromSshConfig("Host a.b\nHost a-b\n");
  assert.deepEqual(j.map((x) => x.name), ["a-b", "a-b-2"]);
});

test("an unfollowed Include is reported, not silently skipped", () => {
  const warnings: string[] = [];
  fromSshConfig("Include ~/.ssh/work/*\nHost x\n", (m) => warnings.push(m));
  assert.ok(warnings.some((w) => w.includes("Include")), warnings.join("\n"));
});

test("keywords are case-insensitive and = separates as well as a space", () => {
  const [j] = fromSshConfig("HOST one\n  hostname=10.0.0.7\n  USER  bob\n");
  assert.equal(j.host, "10.0.0.7");
  assert.equal(j.user, "bob");
});

test("the toml round-trips and leaves out what wasn't set", () => {
  const out = toToml(fromSshConfig("Host one\n  HostName 10.0.0.7\n"));
  assert.match(out, /\[jack\.one\]/);
  assert.match(out, /host = "10\.0\.0\.7"/);
  assert.doesNotMatch(out, /user/, "an unset key must not appear at all");
});
