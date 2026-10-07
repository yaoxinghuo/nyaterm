import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const example = path.join(root, "plugins/examples/native-counter");
const result = spawnSync(
  "cargo",
  ["build", "--manifest-path", path.join(example, "Cargo.toml")],
  { cwd: root, stdio: "inherit", windowsHide: true },
);
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const os = { win32: "windows", darwin: "macos", linux: "linux" }[
  process.platform
];
const arch = { x64: "x86_64", arm64: "aarch64" }[process.arch];
if (!os || !arch) throw new Error("Unsupported native example platform");
const executable = `nyaterm-native-counter${process.platform === "win32" ? ".exe" : ""}`;
const directory = path.join(root, "temp/plugins/native-counter");
mkdirSync(path.join(directory, "bin"), { recursive: true });
copyFileSync(
  path.join(example, "target/debug", executable),
  path.join(directory, "bin", executable),
);
const { version } = JSON.parse(
  readFileSync(path.join(root, "package.json"), "utf8"),
);
writeFileSync(
  path.join(directory, "manifest.json"),
  JSON.stringify(
    {
      manifestVersion: 1,
      id: "nyaterm.native-counter",
      name: "Native Counter",
      version: "1.0.1",
      description:
        "Demonstrates a persistent native command. Each invocation increments the process counter.",
      publisher: "NyaTerm example",
      engine: `>=${version}, <2.0.0`,
      permissions: ["native"],
      backend: {
        transport: "stdio-jsonl",
        executables: { [`${os}-${arch}`]: `bin/${executable}` },
      },
      contributions: {
        commands: [
          {
            id: "count",
            title: "Increment native counter",
            method: "command/count",
            menus: ["terminal", "connection"],
          },
        ],
      },
    },
    null,
    2,
  ),
);
process.stdout.write(
  `Native example prepared in ${directory}\nPackage with: pnpm plugin:pack temp/plugins/native-counter temp/plugins/native-counter.nyap\n`,
);
