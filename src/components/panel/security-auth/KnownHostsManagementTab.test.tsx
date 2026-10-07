import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { KnownHostEntry } from "@/types/global";
import { KnownHostsManagementTab } from "./KnownHostsManagementTab";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("react-i18next", () => {
  const t = (key: string) => key;
  return { useTranslation: () => ({ t }) };
});

const entries: KnownHostEntry[] = [
  {
    id: "known_hosts/one",
    marker: "@cert-authority",
    hostIdentifier: "one.example.com",
    hostPatterns: ["one.example.com", "two.example.com"],
    keyType: "ssh-ed25519",
    fingerprint: "SHA256:abc",
  },
  {
    id: "known_hosts/two",
    hostIdentifier: "broken.example.com",
    hostPatterns: ["broken.example.com"],
    keyType: "ssh-rsa",
    fingerprint: null,
  },
];

function deferredVoid() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("KnownHostsManagementTab", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("renders known hosts and deletes one exact stored record", async () => {
    const onCountChange = vi.fn();
    let loadCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_known_hosts") {
        loadCount += 1;
        return Promise.resolve(loadCount === 1 ? entries : entries.slice(1));
      }
      return Promise.resolve(undefined);
    });

    render(<KnownHostsManagementTab onCountChange={onCountChange} />);

    expect(
      await screen.findByText("@cert-authority one.example.com,two.example.com"),
    ).not.toBeNull();
    expect(screen.getByText(/SHA256:abc/)).not.toBeNull();
    expect(screen.getByText(/knownHosts\.fingerprintUnavailable/)).not.toBeNull();
    expect(onCountChange).toHaveBeenLastCalledWith(2);

    fireEvent.click(screen.getAllByLabelText("knownHosts.deleteEntry")[0]);
    fireEvent.click(screen.getByRole("button", { name: "common.delete" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("delete_known_host", { id: "known_hosts/one" });
      expect(onCountChange).toHaveBeenLastCalledWith(1);
    });
    expect(screen.queryByText("@cert-authority one.example.com,two.example.com")).toBeNull();
    expect(screen.getByText("broken.example.com")).not.toBeNull();
  });

  it("prevents duplicate delete requests while deletion is pending", async () => {
    const pending = deferredVoid();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_known_hosts") return Promise.resolve(entries);
      if (command === "delete_known_host") return pending.promise;
      return Promise.resolve(undefined);
    });

    render(<KnownHostsManagementTab />);
    await screen.findByText("@cert-authority one.example.com,two.example.com");

    fireEvent.click(screen.getAllByLabelText("knownHosts.deleteEntry")[0]);
    const confirm = screen.getByRole("button", { name: "common.delete" }) as HTMLButtonElement;
    fireEvent.click(confirm);

    expect(confirm.disabled).toBe(true);
    fireEvent.click(confirm);
    expect(invokeMock.mock.calls.filter(([command]) => command === "delete_known_host")).toHaveLength(1);

    pending.resolve();
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "common.delete" })).toBeNull();
    });
  });

  it("clears all known hosts and refreshes the empty state and count", async () => {
    const onCountChange = vi.fn();
    let loadCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_known_hosts") {
        loadCount += 1;
        return Promise.resolve(loadCount === 1 ? entries : []);
      }
      return Promise.resolve(undefined);
    });

    render(<KnownHostsManagementTab onCountChange={onCountChange} />);
    await screen.findByText("@cert-authority one.example.com,two.example.com");

    fireEvent.click(screen.getByRole("button", { name: "knownHosts.clearAll" }));
    await screen.findByText("knownHosts.clearTitle");
    fireEvent.click(screen.getByRole("button", { name: "knownHosts.clearAll" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("clear_known_hosts");
      expect(onCountChange).toHaveBeenLastCalledWith(0);
    });
    expect(screen.getByText("knownHosts.noEntries")).not.toBeNull();
  });

  it("prevents duplicate clear requests while clearing is pending", async () => {
    const pending = deferredVoid();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_known_hosts") return Promise.resolve(entries);
      if (command === "clear_known_hosts") return pending.promise;
      return Promise.resolve(undefined);
    });

    render(<KnownHostsManagementTab />);
    await screen.findByText("@cert-authority one.example.com,two.example.com");

    fireEvent.click(screen.getByRole("button", { name: "knownHosts.clearAll" }));
    await screen.findByText("knownHosts.clearTitle");
    const confirm = screen.getByRole("button", {
      name: "knownHosts.clearAll",
    }) as HTMLButtonElement;
    fireEvent.click(confirm);

    expect(confirm.disabled).toBe(true);
    fireEvent.click(confirm);
    expect(invokeMock.mock.calls.filter(([command]) => command === "clear_known_hosts")).toHaveLength(1);

    pending.resolve();
    await waitFor(() => {
      expect(screen.queryByText("knownHosts.clearTitle")).toBeNull();
    });
  });

  it("allows clearing when only hidden raw known-host records remain", async () => {
    const onCountChange = vi.fn();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_known_hosts") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<KnownHostsManagementTab onCountChange={onCountChange} />);

    await screen.findByText("knownHosts.noEntries");
    expect(onCountChange).toHaveBeenLastCalledWith(0);
    fireEvent.click(screen.getByRole("button", { name: "knownHosts.clearAll" }));
    await screen.findByText("knownHosts.clearTitle");
    fireEvent.click(screen.getByRole("button", { name: "knownHosts.clearAll" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("clear_known_hosts");
      expect(onCountChange).toHaveBeenLastCalledWith(0);
    });
  });
});
