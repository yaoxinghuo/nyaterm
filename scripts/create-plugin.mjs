import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const args = process.argv.slice(2);
const [id, destination, option, template = "ui"] = args;
if (
  args.length < 2 ||
  args.length > 4 ||
  (option && option !== "--template") ||
  (option && args.length !== 4) ||
  !["ui", "rust"].includes(template)
) {
  process.stderr.write(
    "Usage: pnpm plugin:create <publisher.plugin> <directory> [--template ui|rust]\n",
  );
  process.exit(1);
}
if (
  !/^[a-z0-9][a-z0-9._-]{0,127}$/.test(id) ||
  !id.includes(".") ||
  id.includes("..")
) {
  throw new Error(
    "Use a namespaced lowercase plugin ID, for example example.hello",
  );
}
const target = path.resolve(destination);
if (existsSync(target))
  throw new Error(`Destination already exists: ${target}`);
const { version } = JSON.parse(
  readFileSync(path.join(root, "package.json"), "utf8"),
);
const distributed = !existsSync(
  path.join(root, "plugins/sdk/rust/nyaterm-plugin-sdk/Cargo.toml"),
);
const sdkDependency = distributed
  ? '{ git = "https://github.com/nyakang/nyaterm", rev = "ac014d96672b5c33736bdc364be9c21b44ec95dc" }'
  : `{ path = ${JSON.stringify(path.relative(target, path.join(root, "plugins/sdk/rust/nyaterm-plugin-sdk")).split(path.sep).join("/"))} }`;
const manifest = {
  manifestVersion: 1,
  id,
  name: id,
  version: "1.0.0",
  description: "A NyaTerm plugin starter",
  publisher: id.split(".")[0],
  engine: `>=${version}, <2.0.0`,
  permissions: template === "rust" ? ["native"] : ["session.read"],
  contributions: {
    panels: [{ id: "main", title: id, entry: "ui/index.html" }],
    commands: [
      {
        id: "hello",
        title: `Open ${id}`,
        panel: "main",
        menus: ["terminal", "connection"],
      },
    ],
  },
};
if (template === "rust") {
  manifest.backend = { transport: "stdio-jsonl", executables: {} };
  manifest.contributions.commands.push({
    id: "run",
    title: "Run native hello",
    method: "command/hello",
    menus: ["terminal", "connection"],
  });
}
mkdirSync(path.dirname(target), { recursive: true });
const stage = mkdtempSync(path.join(path.dirname(target), ".nyaterm-plugin-"));
try {
  const put = (file, content) => {
    const output = path.join(stage, file);
    mkdirSync(path.dirname(output), { recursive: true });
    writeFileSync(output, content);
  };
  put("manifest.json", `${JSON.stringify(manifest, null, 2)}\n`);
  put(".gitignore", "target/\nbin/\n*.nyap\n");
  put(
    "ui/index.html",
    readFileSync(path.join(root, "plugins/templates/ui/index.html"), "utf8"),
  );
  put(
    "ui/main.js",
    template === "rust"
      ? 'document.getElementById("run").onclick = async () => {\n  const output = document.getElementById("output");\n  try { await NyaTerm.ready; output.textContent = JSON.stringify(await NyaTerm.backend("ui/hello"), null, 2); }\n  catch (error) { output.textContent = error.message; }\n};\n'
      : 'document.getElementById("run").onclick = async () => {\n  const output = document.getElementById("output");\n  try { await NyaTerm.ready; output.textContent = JSON.stringify(await NyaTerm.session(), null, 2); }\n  catch (error) { output.textContent = error.message; }\n};\n',
  );
  put(
    "ui/nyaterm-sdk.js",
    readFileSync(path.join(root, "plugins/sdk/nyaterm.js")),
  );
  put(
    "ui/nyaterm-sdk.d.ts",
    readFileSync(path.join(root, "plugins/sdk/nyaterm.d.ts")),
  );
  if (template === "rust") {
    put(
      "Cargo.toml",
      `[package]\nname = "nyaterm-plugin-starter"\nversion = "1.0.0"\nedition = "2024"\nrust-version = "1.94"\n\n[dependencies]\nnyaterm-plugin-sdk = ${sdkDependency}\ntokio = { version = "1", features = ["rt", "macros"] }\n`,
    );
    put(
      "src/main.rs",
      readFileSync(path.join(root, "plugins/templates/rust/main.rs")),
    );
    put(
      "build.mjs",
      readFileSync(path.join(root, "plugins/templates/rust/build.mjs")),
    );
  }
  const packer = path.join(root, "scripts/package-plugin.mjs");
  const packCommand = distributed
    ? `nyaterm-plugin pack . ../${id}.nyap`
    : `node ${JSON.stringify(packer)} . ../${id}.nyap`;
  put(
    "README.md",
    `# ${id}\n\nGenerated from the NyaTerm ${template} template.\n\n${template === "rust" ? "1. Install Rust 1.94 or newer, then run `node build.mjs` here. This builds the SDK backend and fills the current platform executable in manifest.json.\n2." : "1."} Package from this directory:\n\n\`\`\`sh\n${packCommand}\n\`\`\`\n\nInstall the .nyap from NyaTerm's Plugins panel, review permissions, enable it and open its panel. ${template === "rust" ? "The native command reuses the process; check status/logs in Plugins. Stop the backend to reset its counter. Never print protocol data to stdout: use Context::log or stderr. The standalone CLI pins the Rust SDK to a public Git commit; source-checkout templates use a local SDK dependency. For framed transport change backend.transport to stdio-framed; the SDK selects the transport from the host environment. Add other-platform binaries under bin/ and their executable entries before packing." : "Select a connected terminal before inspecting its session. The UI runs in an isolated iframe; use the SDK instead of Tauri APIs or direct file/network access."}\n\nEdit manifest.json to add permissions and contributions. UI scripts and styles must be local; use textContent for host/plugin data. Packaging includes only manifest.json, ui/, assets/ and bin/; target and source are excluded. The packager injects the current UI SDK. To update an installed plugin, bump manifest.version (and Cargo version for Rust) because released versions are immutable.\n`,
  );
  renameSync(stage, target);
} catch (error) {
  rmSync(stage, { recursive: true, force: true });
  throw error;
}
process.stdout.write(
  `Created ${template} plugin ${id} at ${target}\nSee README.md for build and packaging instructions.\n`,
);
