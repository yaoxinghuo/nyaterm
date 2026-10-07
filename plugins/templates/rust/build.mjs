import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("./", import.meta.url));
const os = { win32: "windows", darwin: "macos", linux: "linux" }[
  process.platform
];
const arch = { x64: "x86_64", arm64: "aarch64" }[process.arch];
if (!os || !arch)
  throw new Error(
    "Unsupported platform; build and add your executable manually",
  );
const result = spawnSync(
  "cargo",
  [
    "build",
    "--release",
    "--manifest-path",
    path.join(root, "Cargo.toml"),
    "--target-dir",
    path.join(root, "target"),
  ],
  { cwd: root, stdio: "inherit", windowsHide: true },
);
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const executable = `nyaterm-plugin-starter${process.platform === "win32" ? ".exe" : ""}`;
const directory = path.join(root, "bin", `${os}-${arch}`);
mkdirSync(directory, { recursive: true });
copyFileSync(
  path.join(root, "target/release", executable),
  path.join(directory, executable),
);
const manifestPath = path.join(root, "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
manifest.backend.executables[`${os}-${arch}`] =
  `bin/${os}-${arch}/${executable}`;
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
process.stdout.write("Built native backend; the plugin is ready to package.\n");
