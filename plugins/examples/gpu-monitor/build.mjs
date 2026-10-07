import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const root = fileURLToPath(new URL("./", import.meta.url));
const repository = path.resolve(root, "../../..");
// Execute on Linux even when this project was created/checked out on Windows.
const probePath = path.join(root, "assets/probes/gpu.sh");
const probe = readFileSync(probePath, "utf8").replace(/\r\n/g, "\n");
writeFileSync(probePath, probe);
const translations = {};
for (const language of ["en", "zh-CN", "zh-TW", "ko"]) {
  const locale = JSON.parse(
    readFileSync(
      path.join(repository, "src/i18n/locales", `${language}.json`),
      "utf8",
    ),
  );
  translations[language] = {
    translation: {
      gpuMonitor: locale.gpuMonitor,
      panel: { gpuMonitor: locale.panel.gpuMonitor },
      common: {
        refresh: locale.common.refresh,
        loading: locale.common.loading,
      },
    },
  };
}
writeFileSync(
  path.join(root, "frontend/translations.json"),
  `${JSON.stringify(translations)}\n`,
);
const typecheck = spawnSync(
  process.execPath,
  [
    path.join(repository, "node_modules/typescript/bin/tsc"),
    "-p",
    path.join(root, "frontend/tsconfig.json"),
    "--noEmit",
  ],
  { stdio: "inherit", windowsHide: true },
);
if (typecheck.error) throw typecheck.error;
if (typecheck.status !== 0) process.exit(typecheck.status ?? 1);
await build({
  configFile: false,
  root,
  base: "./",
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.join(repository, "src") } },
  build: {
    outDir: "ui",
    emptyOutDir: true,
    cssCodeSplit: false,
    rollupOptions: {
      input: path.join(root, "frontend/main.tsx"),
      output: {
        format: "iife",
        name: "NyaTermGpu",
        inlineDynamicImports: true,
        entryFileNames: "app.js",
        assetFileNames: "[name][extname]",
      },
    },
  },
});
const css = "style.css";
writeFileSync(
  path.join(root, "ui/index.html"),
  `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="${css}"><title>GPU Monitor</title></head><body><div id="root"></div><script src="nyaterm-sdk.js"></script><script src="app.js"></script></body></html>\n`,
);
copyFileSync(
  path.join(repository, "plugins/sdk/nyaterm.js"),
  path.join(root, "ui/nyaterm-sdk.js"),
);
const os = { win32: "windows", linux: "linux", darwin: "macos" }[
  process.platform
];
const arch = { x64: "x86_64", arm64: "aarch64" }[process.arch];
if (!os || !arch) throw new Error("Unsupported platform");
const result = spawnSync(
  "cargo",
  [
    "build",
    "--release",
    "--manifest-path",
    path.join(root, "backend/Cargo.toml"),
    "--target-dir",
    path.join(root, "backend/target"),
  ],
  { stdio: "inherit", windowsHide: true },
);
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const executable = `nyaterm-plugin-gpu${process.platform === "win32" ? ".exe" : ""}`;
const relative = `bin/${os}-${arch}/${executable}`;
mkdirSync(path.dirname(path.join(root, relative)), { recursive: true });
copyFileSync(
  path.join(root, "backend/target/release", executable),
  path.join(root, relative),
);
const manifestPath = path.join(root, "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
manifest.backend.executables = { [`${os}-${arch}`]: relative };
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
process.stdout.write("GPU plugin built; package it with pnpm plugin:pack.\n");
