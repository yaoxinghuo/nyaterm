// Exercise the built classic-script entry and real GPU components without Tauri.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM, VirtualConsole } from "jsdom";

const root = new URL("./", import.meta.url);
const html = readFileSync(new URL("ui/index.html", root), "utf8");
assert(!html.includes('type="module"'));
for (const match of html.matchAll(/(?:src|href)="([^"]+)"/g)) {
  assert(!/^(?:[a-z]+:|\/)/i.test(match[1]), "Assets must use relative paths");
  assert(readFileSync(new URL(`ui/${match[1]}`, root)).length > 0);
}
const errors = [];
const virtualConsole = new VirtualConsole();
virtualConsole.on("jsdomError", (error) => errors.push(error.message));
const dom = new JSDOM(html.replace(/<script[^>]*><\/script>/g, ""), {
  url: "https://gpu-plugin.invalid/ui/index.html",
  runScripts: "dangerously",
  pretendToBeVisual: true,
  virtualConsole,
});
const { window } = dom;
const calls = [];
let revision = 1;
const snapshot = () => ({
  revision: revision++,
  sessionId: "ssh-1",
  error: false,
  refreshing: false,
  paused: false,
  overview: {
    available: true,
    driver_version: "550.54",
    cuda_version: "12.4",
    gpus: [
      {
        index: 0,
        uuid: "GPU-a",
        name: "NVIDIA RTX 4090",
        temperature_c: 43,
        utilization_gpu_percent: 77,
        utilization_memory_percent: 31,
        memory_total_mb: 24564,
        memory_used_mb: 12345,
        memory_free_mb: 12219,
        power_draw_w: 180.5,
        power_limit_w: 450,
        fan_speed_percent: 46,
        pstate: "P2",
      },
    ],
    processes: [
      {
        gpu_uuid: "GPU-a",
        gpu_index: 0,
        pid: 4242,
        process_name: "python",
        used_memory_mb: 2048,
      },
      {
        gpu_uuid: "GPU-a",
        gpu_index: 0,
        pid: 4243,
        process_name: "node",
        used_memory_mb: 1024,
      },
    ],
  },
});
const context = {
  pluginId: "nyaterm.gpu",
  version: "1.0.0",
  language: "en",
  monitorIntervalSeconds: 3,
  theme: {
    "--background": "#111111",
    "--foreground": "#eeeeee",
    "--df-bg-panel": "#111111",
  },
};
let parent;
const send = (data) =>
  window.dispatchEvent(
    new window.MessageEvent("message", { source: parent, data }),
  );
parent = {
  postMessage(message) {
    queueMicrotask(() => {
      if (message.type === "nyaterm-plugin-ready")
        return send({ type: "nyaterm-plugin-context", context });
      if (message.type !== "nyaterm-plugin-request") return;
      calls.push(message.method);
      const result =
        message.method === "host/session"
          ? { id: "ssh-1" }
          : message.method === "host/monitoring/subscribe"
            ? { subscriptionId: "gpu-sub", snapshot: snapshot() }
            : null;
      send({ type: "nyaterm-plugin-response", id: message.id, result });
    });
  },
};
Object.defineProperty(window, "parent", { value: parent });
window.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
window.eval(readFileSync(new URL("ui/nyaterm-sdk.js", root), "utf8"));
window.eval(readFileSync(new URL("ui/app.js", root), "utf8"));
const until = async (predicate) => {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  assert.fail(`UI did not settle: ${window.document.body.textContent}`);
};
await until(() => window.document.body.textContent.includes("NVIDIA RTX 4090"));
const text = () => window.document.body.textContent;
assert(text().includes("550.54") && text().includes("12.4"));
assert(text().includes("43 C") && text().includes("77%"));
await until(() => text().includes("python") && text().includes("node"));
assert(
  text().indexOf("python") < text().indexOf("node"),
  "Processes are ordered by memory",
);
const input = window.document.querySelector("input");
const setValue = Object.getOwnPropertyDescriptor(
  window.HTMLInputElement.prototype,
  "value",
).set;
setValue.call(input, "no-such-process");
input.dispatchEvent(new window.Event("input", { bubbles: true }));
const searchLocale = JSON.parse(
  readFileSync(new URL("../../../src/i18n/locales/en.json", root), "utf8"),
);
await until(() => text().includes(searchLocale.gpuMonitor.noMatches));
setValue.call(input, "");
input.dispatchEvent(new window.Event("input", { bubbles: true }));
await until(() => text().includes("python"));
const button = (label) =>
  [...window.document.querySelectorAll("button")].find(
    (element) => element.getAttribute("aria-label") === label,
  );
const english = JSON.parse(
  readFileSync(new URL("../../../src/i18n/locales/en.json", root), "utf8"),
);
button(english.gpuMonitor.details).click();
await until(() => text().includes("46%"));
button(english.common.refresh).click();
await until(() => calls.includes("host/monitoring/refresh"));
const subscriptions = calls.filter(
  (method) => method === "host/monitoring/subscribe",
).length;
for (const language of ["zh-CN", "zh-TW", "ko", "en"]) {
  context.language = language;
  send({ type: "nyaterm-plugin-context", context });
  const locale = JSON.parse(
    readFileSync(
      new URL(`../../../src/i18n/locales/${language}.json`, root),
      "utf8",
    ),
  );
  await until(() => text().includes(locale.panel.gpuMonitor));
}
assert.equal(
  calls.filter((method) => method === "host/monitoring/subscribe").length,
  subscriptions,
  "Theme/language updates must preserve the subscription",
);
assert.equal(
  window.document.documentElement.style.getPropertyValue("--background"),
  "#111111",
);
send({
  type: "nyaterm-plugin-monitor",
  subscriptionId: "gpu-sub",
  snapshot: { ...snapshot(), error: true, paused: true },
});
await until(
  () =>
    text().includes(english.gpuMonitor.paused) &&
    text().includes(english.gpuMonitor.error),
);
assert(
  text().includes("NVIDIA RTX 4090"),
  "A temporary failure preserves data",
);
assert.deepEqual(errors, []);
window.dispatchEvent(new window.Event("pagehide"));
window.close();
process.stdout.write(
  "Built GPU UI passed: relative classic assets, full metrics/processes, refresh, four languages, theme and retained error/paused state.\n",
);
