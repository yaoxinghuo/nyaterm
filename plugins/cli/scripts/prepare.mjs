import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const cli = path.join(root, "plugins/cli");
const assets = path.join(cli, "assets");
mkdirSync(path.join(assets, "scripts"), { recursive: true });
mkdirSync(path.join(assets, "plugins/sdk"), { recursive: true });
copyFileSync(
  path.join(root, "scripts/create-plugin.mjs"),
  path.join(assets, "scripts/create-plugin.mjs"),
);
for (const file of ["nyaterm.js", "nyaterm.d.ts"])
  copyFileSync(
    path.join(root, "plugins/sdk", file),
    path.join(assets, "plugins/sdk", file),
  );
cpSync(
  path.join(root, "plugins/templates"),
  path.join(assets, "plugins/templates"),
  { recursive: true },
);
writeFileSync(
  path.join(assets, "package.json"),
  `${JSON.stringify({ type: "module", version: JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")).version }, null, 2)}\n`,
);
chmodSync(path.join(cli, "bin/nyaterm-plugin.mjs"), 0o755);
const os = { win32: "windows", darwin: "macos", linux: "linux" }[
  process.platform
];
const arch = { x64: "x86_64", arm64: "aarch64" }[process.arch];
const native = path.join(cli, "native");
mkdirSync(native, { recursive: true });
const name = `nyaterm-plugin-${os}-${arch}${process.platform === "win32" ? ".exe" : ""}`;
const compiled =
  process.env.NYATERM_PLUGIN_BINARY ??
  path.join(
    root,
    "src-tauri/crates/nyaterm-plugin-runtime/target/release",
    `nyaterm-plugin${process.platform === "win32" ? ".exe" : ""}`,
  );
if (existsSync(compiled)) {
  copyFileSync(compiled, path.join(native, name));
} else if (!existsSync(path.join(native, name))) {
  throw new Error(
    "Build the native CLI first: cargo build --release --manifest-path src-tauri/crates/nyaterm-plugin-runtime/Cargo.toml --bin nyaterm-plugin",
  );
}
for (const platform of [
  "windows-x86_64",
  "windows-aarch64",
  "linux-x86_64",
  "linux-aarch64",
  "macos-x86_64",
  "macos-aarch64",
]) {
  const file = path.join(
    native,
    `nyaterm-plugin-${platform}${platform.startsWith("windows") ? ".exe" : ""}`,
  );
  if (process.env.NYATERM_CLI_RELEASE === "1" && !existsSync(file))
    throw new Error(`Release is missing native binary: ${platform}`);
  if (existsSync(file)) chmodSync(file, 0o755);
}
