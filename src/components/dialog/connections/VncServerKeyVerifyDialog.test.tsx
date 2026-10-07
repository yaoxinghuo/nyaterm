import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import {
  VncServerKeyVerifyDialog,
  type VncServerKeyVerifyRequest,
} from "./VncServerKeyVerifyDialog";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/logger", () => ({ logger: { info: vi.fn(), error: vi.fn() } }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const request: VncServerKeyVerifyRequest = {
  requestId: "key-request",
  sessionId: "vnc-session",
  host: "pi.local",
  port: 5900,
  fingerprint: "SHA256:key",
  keyBits: 1024,
  knownHostStatus: "changed",
  targetWindowLabel: "main-second",
};

beforeEach(() => {
  mocks.invoke.mockReset().mockResolvedValue(undefined);
});

it("warns about changed keys and treats closing the dialog as Reject", async () => {
  const onDone = vi.fn();
  render(<VncServerKeyVerifyDialog request={request} onDone={onDone} />);
  expect(screen.getByText("settings.vncServerKeyVerifyWarning")).not.toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  await waitFor(() => expect(onDone).toHaveBeenCalledWith(request.requestId));
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
    "respond_vnc_server_key",
    {
      requestId: request.requestId,
      accepted: false,
    },
  );
});

it("clears a dialog when its request has expired before the response arrives", async () => {
  mocks.invoke.mockRejectedValue(
    new Error("No pending VNC server key request"),
  );
  const onDone = vi.fn();
  render(<VncServerKeyVerifyDialog request={request} onDone={onDone} />);
  fireEvent.click(
    screen.getByRole("button", { name: "settings.vncServerKeyAccept" }),
  );
  await waitFor(() => expect(onDone).toHaveBeenCalledWith(request.requestId));
});
