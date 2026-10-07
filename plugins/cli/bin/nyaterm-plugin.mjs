#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const [command, ...args] = process.argv.slice(2);
const os = { win32: "windows", darwin: "macos", linux: "linux" }[
  process.platform
];
const arch = { x64: "x86_64", arm64: "aarch64" }[process.arch];
const binary = path.join(
  root,
  "native",
  `nyaterm-plugin-${os}-${arch}${process.platform === "win32" ? ".exe" : ""}`,
);
const appVersion = JSON.parse(
  readFileSync(path.join(root, "assets/package.json"), "utf8"),
).version;
let executable, parameters;
if (command === "create") {
  executable = process.execPath;
  parameters = [path.join(root, "assets/scripts/create-plugin.mjs"), ...args];
} else if (
  (command === "pack" && [2, 3].includes(args.length)) ||
  (command === "inspect" && [1, 2].includes(args.length))
) {
  if (!os || !arch || !existsSync(binary))
    throw new Error(
      "This CLI package has no binary for your platform. Supported: Windows, Linux and macOS x86_64/aarch64.",
    );
  executable = binary;
  parameters = [command, ...args];
  if (args.length === (command === "pack" ? 2 : 1)) parameters.push(appVersion);
} else {
  process.stderr.write(
    "Usage: nyaterm-plugin create <publisher.plugin> <directory> [--template ui|rust]\n       nyaterm-plugin pack <directory> <output.nyap> [app-version]\n       nyaterm-plugin inspect <package.nyap> [app-version]\n",
  );
  process.exit(1);
}
const result = spawnSync(executable, parameters, {
  stdio: "inherit",
  windowsHide: true,
});
if (result.error) process.stderr.write(`${result.error.message}\n`);
process.exit(result.status ?? 1);
