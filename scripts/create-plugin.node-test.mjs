import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = fileURLToPath(new URL("./create-plugin.mjs", import.meta.url));
function withDirectory(fn) {
  const directory = mkdtempSync(path.join(os.tmpdir(), "nyaterm scaffold "));
  try {
    fn(directory);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
test("generates UI source and refuses to overwrite an existing project", () =>
  withDirectory((directory) => {
    const target = path.join(directory, "ui");
    assert.equal(
      spawnSync(process.execPath, [script, "example.hello", target]).status,
      0,
    );
    const original = readFileSync(path.join(target, "manifest.json"), "utf8");
    assert.equal(JSON.parse(original).permissions[0], "session.read");
    assert.ok(existsSync(path.join(target, "ui/nyaterm-sdk.js")));
    assert.notEqual(
      spawnSync(process.execPath, [script, "example.changed", target]).status,
      0,
    );
    assert.equal(
      readFileSync(path.join(target, "manifest.json"), "utf8"),
      original,
    );
  }));
test("generates Rust source with a usable SDK path outside the repository", () =>
  withDirectory((directory) => {
    const target = path.join(directory, "rust");
    assert.equal(
      spawnSync(process.execPath, [
        script,
        "example.hello",
        target,
        "--template",
        "rust",
      ]).status,
      0,
    );
    const cargo = readFileSync(path.join(target, "Cargo.toml"), "utf8");
    const sdk = JSON.parse(cargo.match(/path = (".*")/)[1]);
    assert.ok(existsSync(path.resolve(target, sdk, "Cargo.toml")));
    assert.ok(
      readFileSync(path.join(target, "src/main.rs"), "utf8").includes(
        "Server::from_env",
      ),
    );
    assert.ok(existsSync(path.join(target, "build.mjs")));
  }));
test("rejects invalid IDs and template arguments without creating files", () =>
  withDirectory((directory) => {
    const target = path.join(directory, "invalid");
    for (const args of [
      ["../escape", target],
      ["no-namespace", target],
      ["example.test", target, "--template", "unknown"],
      ["example.test", target, "--template"],
    ]) {
      assert.notEqual(spawnSync(process.execPath, [script, ...args]).status, 0);
      assert.equal(existsSync(target), false);
    }
  }));
