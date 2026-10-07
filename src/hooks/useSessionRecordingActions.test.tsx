import { act, renderHook } from "@testing-library/react";
import type { TFunction } from "i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RecordingSettings } from "@/types/global";
import { useSessionRecordingActions } from "./useSessionRecordingActions";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  downloadDir: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));
vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/path", () => ({ downloadDir: mocks.downloadDir }));
vi.mock("sonner", () => ({ toast: { success: mocks.success, error: mocks.error } }));
vi.mock("@/lib/logger", () => ({ logger: { error: vi.fn() } }));

function renderActions() {
  const options: Parameters<typeof useSessionRecordingActions>[0] = {
    recording: {
      base_path: "E:\\logs",
      include_io_labels: true,
      include_timestamps: false,
    } as RecordingSettings,
    recordingSessions: new Set(),
    liveSessionsById: null,
    refreshRecordingStatuses: vi.fn().mockResolvedValue(undefined),
    t: ((key: string) => key) as TFunction,
  };
  return { ...renderHook(useSessionRecordingActions, { initialProps: options }), options };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.invoke.mockReset().mockResolvedValue("saved.log");
  mocks.downloadDir.mockResolvedValue("C:\\Downloads\\");
});

describe("useSessionRecordingActions", () => {
  it("starts the requested recording mode and stops an existing recording after state refresh", async () => {
    const { result, rerender, options } = renderActions();
    await act(() => result.current.handleToggleSessionRecordingById("ssh-1", "raw"));
    expect(mocks.invoke).toHaveBeenLastCalledWith("start_recording", {
      request: { sessionId: "ssh-1", mode: "raw", explicitPath: null },
    });
    rerender({ ...options, recordingSessions: new Set(["ssh-1"]) });
    await act(() => result.current.handleToggleSessionRecordingById("ssh-1"));
    expect(mocks.invoke).toHaveBeenLastCalledWith("stop_recording", { sessionId: "ssh-1" });
    expect(options.refreshRecordingStatuses).toHaveBeenCalledTimes(2);
    expect(mocks.success).toHaveBeenLastCalledWith("recording.saved");
  });

  it("exports to the configured directory with a safe name and uses updated export settings", async () => {
    const { result, rerender, options } = renderActions();
    await act(() => result.current.handleSaveSessionTranscriptById("ssh-1", "主机 / prod"));
    expect(mocks.invoke).toHaveBeenLastCalledWith("save_session_transcript", {
      sessionId: "ssh-1",
      filePath: expect.stringMatching(/^E:\\logs\/session-主机_prod-.*\.log$/),
      includeIoLabels: true,
      includeTimestamps: false,
    });
    expect(mocks.downloadDir).not.toHaveBeenCalled();
    rerender({
      ...options,
      recording: {
        ...options.recording,
        base_path: "",
        include_io_labels: false,
        include_timestamps: true,
      },
    });
    await act(() => result.current.handleSaveSessionTranscriptById("ssh-1", "Host"));
    expect(mocks.invoke).toHaveBeenLastCalledWith("save_session_transcript", {
      sessionId: "ssh-1",
      filePath: expect.stringMatching(/^C:\\Downloads\\session-Host-.*\.log$/),
      includeIoLabels: false,
      includeTimestamps: true,
    });
  });

  it("reports a failed recording without refreshing statuses or showing success", async () => {
    mocks.invoke.mockRejectedValueOnce(new Error("Disk full"));
    const { result, options } = renderActions();
    await act(() => result.current.handleToggleSessionRecordingById("ssh-1"));
    expect(options.refreshRecordingStatuses).not.toHaveBeenCalled();
    expect(mocks.success).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith("recording.startFailed");
  });
});
