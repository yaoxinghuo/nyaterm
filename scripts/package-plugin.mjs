import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
if (args.length !== 2) {
  process.stderr.write(
    "Usage: pnpm plugin:pack <plugin-directory> <output.nyap>\n",
  );
  process.exit(1);
}
const version = JSON.parse(
  readFileSync(path.join(root, "package.json"), "utf8"),
).version;
const result = spawnSync(
  "cargo",
  [
    "run",
    "--manifest-path",
    path.join(root, "src-tauri/crates/nyaterm-plugin-runtime/Cargo.toml"),
    "--bin",
    "nyaterm-plugin",
    "--",
    "pack",
    path.resolve(args[0]),
    path.resolve(args[1]),
    version,
  ],
  { cwd: root, stdio: "inherit", windowsHide: true },
);
if (result.error) process.stderr.write(`${result.error.message}\n`);
process.exit(result.status ?? 1);
