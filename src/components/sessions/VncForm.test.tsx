import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { VncForm } from "./VncForm";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function props() {
  return {
    host: "vnc.example.com",
    setHost: vi.fn(),
    port: 5900,
    setPort: vi.fn(),
    username: "pi",
    setUsername: vi.fn(),
    passwordId: "",
    setPasswordId: vi.fn(),
    password: "",
    setPassword: vi.fn(),
    hasPassword: false,
    setHasPassword: vi.fn(),
    scaleMode: "fit" as const,
    setScaleMode: vi.fn(),
    securityMode: "auto" as const,
    setSecurityMode: vi.fn(),
    shared: true,
    setShared: vi.fn(),
    viewOnly: false,
    setViewOnly: vi.fn(),
    clipboardEnabled: true,
    setClipboardEnabled: vi.fn(),
    reconnectEnabled: true,
    setReconnectEnabled: vi.fn(),
    reconnectMaxAttempts: 5,
    setReconnectMaxAttempts: vi.fn(),
    proxyId: "",
    setProxyId: vi.fn(),
    proxies: [],
    jumpHostId: "",
    setJumpHostId: vi.fn(),
    jumpHostOptions: [],
  };
}

describe("VncForm", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it("shows the RA2 username in auto mode without applying the classic password hint", () => {
    render(<VncForm {...props()} />);

    expect(screen.getByDisplayValue("5900")).not.toBeNull();
    expect(screen.getByDisplayValue("pi")).not.toBeNull();
    expect(screen.getByText("dialog.vncUsernameHint")).not.toBeNull();
    expect(screen.queryByText("dialog.vncPasswordLimit")).toBeNull();
  });

  it("shows the 8-byte password hint only for classic VNC authentication", () => {
    render(<VncForm {...props()} securityMode="vnc-auth" />);

    expect(screen.getByText("dialog.vncPasswordLimit")).not.toBeNull();
  });

  it("disables the username in None mode and hides authentication hints", () => {
    render(<VncForm {...props()} securityMode="none" />);
    expect((screen.getByDisplayValue("pi") as HTMLInputElement).disabled).toBe(
      true,
    );
    expect(screen.queryByText("dialog.vncUsernameHint")).toBeNull();
    expect(screen.queryByText("dialog.vncPasswordLimit")).toBeNull();
  });

  it("allows switching to saved password selection", () => {
    render(<VncForm {...props()} />);
    fireEvent.click(screen.getByText("dialog.savedPassword"));

    expect(invokeMock).toHaveBeenCalledWith("get_saved_passwords");
  });
});
