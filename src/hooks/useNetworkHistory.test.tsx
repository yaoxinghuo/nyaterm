import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { RemoteStats } from "@/types/global";
import {
  MAX_NETWORK_HISTORY_POINTS,
  useNetworkHistory,
  useNetworkHistorySeries,
} from "./useNetworkHistory";

describe("useNetworkHistory", () => {
  it("removes closed-session history and snapshot references while retaining live history", () => {
    const statsA = remoteStats(10, 1);
    const statsB = remoteStats(20, 2);
    const liveSessions = new Set(["session-a", "session-b"]);
    const { result, rerender } = renderHook(
      ({ sessionId, stats, liveSessionIds }) => useNetworkHistory(sessionId, stats, liveSessionIds),
      {
        initialProps: {
          sessionId: "session-a",
          stats: statsA,
          liveSessionIds: liveSessions as ReadonlySet<string> | null,
        },
      },
    );
    rerender({ sessionId: "session-b", stats: statsB, liveSessionIds: null });
    expect(result.current.getSeries("session-a").summary).toHaveLength(1);
    const historyB = result.current.getSeries("session-b");
    const listener = vi.fn();
    const unsubscribe = result.current.subscribe(listener);

    // Even if stale stats are still supplied, a closed session must stay evicted.
    rerender({ sessionId: "session-a", stats: statsA, liveSessionIds: new Set(["session-b"]) });
    expect(result.current.getSeries("session-a").summary).toHaveLength(0);
    expect(result.current.getSeries("session-b")).toBe(historyB);
    expect(listener).toHaveBeenCalledTimes(1);

    // Reusing the object proves the old deduplication reference was also removed.
    rerender({ sessionId: "session-a", stats: statsA, liveSessionIds: liveSessions });
    expect(result.current.getSeries("session-a").summary).toHaveLength(1);
    unsubscribe();
  });

  it("appends new samples and does not duplicate the same stats object", async () => {
    const first = remoteStats(100, 10, [{ nic: "eth0", rx: 60, tx: 6 }]);
    const { result, rerender } = renderHook(
      ({ stats }) => useNetworkHistory("session-a", stats),
      { initialProps: { stats: first } },
    );

    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(1));
    rerender({ stats: first });
    expect(result.current.getSeries("session-a").summary).toHaveLength(1);

    rerender({ stats: remoteStats(200, 20, [{ nic: "eth0", rx: 120, tx: 12 }]) });
    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(2));
    expect(result.current.getSeries("session-a").summary.map((point) => [point.rx, point.tx])).toEqual([
      [100, 10],
      [200, 20],
    ]);
  });

  it("keeps only the latest 60 points", async () => {
    const { result, rerender } = renderHook(
      ({ stats }) => useNetworkHistory("session-a", stats),
      { initialProps: { stats: remoteStats(0, 0) } },
    );

    for (let i = 1; i <= MAX_NETWORK_HISTORY_POINTS; i += 1) {
      rerender({ stats: remoteStats(i, i * 10) });
    }

    await waitFor(() =>
      expect(result.current.getSeries("session-a").summary).toHaveLength(MAX_NETWORK_HISTORY_POINTS),
    );
    const summary = result.current.getSeries("session-a").summary;
    expect(summary[0]?.rx).toBe(1);
    expect(summary[summary.length - 1]?.rx).toBe(
      MAX_NETWORK_HISTORY_POINTS,
    );
  });

  it("isolates history by session and restores it when switching back", async () => {
    const secondA = remoteStats(20, 2);
    const { result, rerender } = renderHook(
      ({ sessionId, stats }) => useNetworkHistory(sessionId, stats),
      { initialProps: { sessionId: "session-a", stats: remoteStats(10, 1) } },
    );

    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(1));
    rerender({ sessionId: "session-a", stats: secondA });
    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(2));

    rerender({ sessionId: "session-b", stats: remoteStats(30, 3) });
    await waitFor(() => expect(result.current.getSeries("session-b").summary).toHaveLength(1));
    expect(result.current.getSeries("session-b").summary[0]?.rx).toBe(30);

    rerender({ sessionId: "session-a", stats: secondA });
    expect(result.current.getSeries("session-a").summary).toHaveLength(2);
    expect(result.current.getSeries("session-a").summary.map((point) => point.rx)).toEqual([10, 20]);
  });

  it("keeps per-interface histories independent", async () => {
    const { result, rerender } = renderHook(
      ({ stats }) => useNetworkHistory("session-a", stats),
      {
        initialProps: {
          stats: remoteStats(100, 10, [
            { nic: "eth0", rx: 60, tx: 6 },
            { nic: "docker0", rx: 40, tx: 4 },
          ]),
        },
      },
    );

    await waitFor(() =>
      expect(result.current.getSeries("session-a").interfaces.eth0).toHaveLength(1),
    );
    rerender({
      stats: remoteStats(120, 12, [{ nic: "eth0", rx: 120, tx: 12 }]),
    });

    await waitFor(() =>
      expect(result.current.getSeries("session-a").interfaces.eth0).toHaveLength(2),
    );
    const interfaces = result.current.getSeries("session-a").interfaces;
    expect(interfaces.eth0?.map((point) => point.rx)).toEqual([60, 120]);
    expect(interfaces.docker0?.map((point) => point.rx)).toEqual([40]);
  });

  it("records summary from network_summary instead of deriving it from a NIC", async () => {
    const stats = remoteStats(999, 111, [{ nic: "eth0", rx: 10, tx: 20 }]);
    const { result } = renderHook(() => useNetworkHistory("session-a", stats));

    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(1));
    expect(result.current.getSeries("session-a").summary[0]).toEqual(
      expect.objectContaining({ rx: 999, tx: 111 }),
    );
  });

  it("does not trigger an extra collector render when a sample is recorded", async () => {
    let renderCount = 0;
    const { result, rerender } = renderHook(
      ({ stats }) => {
        renderCount += 1;
        return useNetworkHistory("session-a", stats);
      },
      { initialProps: { stats: remoteStats(10, 1) } },
    );

    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(1));
    expect(renderCount).toBe(1);

    rerender({ stats: remoteStats(20, 2) });
    await waitFor(() => expect(result.current.getSeries("session-a").summary).toHaveLength(2));
    expect(renderCount).toBe(2);
  });

  it("updates a subscribed series when the store records a sample", async () => {
    const { result, rerender } = renderHook(
      ({ stats }) => {
        const store = useNetworkHistory("session-a", stats);
        return useNetworkHistorySeries(store, "session-a");
      },
      { initialProps: { stats: remoteStats(10, 1) } },
    );

    await waitFor(() => expect(result.current.summary).toHaveLength(1));
    rerender({ stats: remoteStats(20, 2) });
    await waitFor(() => expect(result.current.summary.map((point) => point.rx)).toEqual([10, 20]));
  });
});

function remoteStats(
  summaryRx: number,
  summaryTx: number,
  networks: Array<{ nic: string; rx: number; tx: number }> = [],
): RemoteStats {
  return {
    system: { hostname: "host", uptime_sec: 100, os: "Linux", arch: "x86_64" },
    load: { load1: 0.1, load5: 0.2, load15: 0.3 },
    cpu: {
      model: "CPU",
      cores: 8,
      usage: 10,
      per_core: [],
      sample_window_ms: 1000,
      usage_source: "aggregate",
    },
    memory: { used: 1024, available: 1024, cached: 0 },
    networks: networks.map((network) => ({
      nic: network.nic,
      state: "up",
      rx_bytes_per_sec: network.rx,
      tx_bytes_per_sec: network.tx,
    })),
    network_summary: {
      rx_bytes_per_sec: summaryRx,
      tx_bytes_per_sec: summaryTx,
    },
    disks: [],
  };
}
