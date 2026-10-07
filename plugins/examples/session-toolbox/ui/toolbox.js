"use strict";

const output = document.getElementById("output");
const error = document.getElementById("error");
const buttons = [...document.querySelectorAll("button")];
async function run(operation) {
  error.textContent = "";
  for (const button of buttons) button.disabled = true;
  try {
    const result = await operation();
    output.textContent =
      typeof result === "string" ? result : JSON.stringify(result, null, 2);
  } catch (failure) {
    error.textContent = failure.message;
  } finally {
    for (const button of buttons) button.disabled = false;
  }
}
document.getElementById("read").addEventListener("click", () =>
  run(async () => {
    await NyaTerm.storage.set("read-lines", 100);
    return NyaTerm.terminal.read(100);
  }),
);
document.getElementById("execute").addEventListener("click", () =>
  run(async () => {
    const command = document.getElementById("command").value;
    return NyaTerm.terminal.execute(command);
  }),
);
document
  .getElementById("file")
  .addEventListener("click", () =>
    run(() => NyaTerm.filesystem.read(document.getElementById("path").value)),
  );
void NyaTerm.ready.then(() =>
  run(async () => {
    const session = await NyaTerm.session();
    document.getElementById("session").textContent = session
      ? `${session.name} (${session.type})`
      : "Select a terminal session and reopen this panel.";
    const lines = (await NyaTerm.storage.get("read-lines")) || 100;
    document.getElementById("read").textContent = `Read ${lines} lines`;
    return "Ready";
  }),
);
