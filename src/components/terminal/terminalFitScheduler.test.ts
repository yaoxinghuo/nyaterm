import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  TerminalFitScheduler,
  TerminalResizeDeduper,
  type TerminalFitDimensions,
} from "./terminalFitScheduler";

vi.mock("@/lib/logger", () => ({
  logger: {
    debug: vi.fn(),
    warn: vi.fn(),
  },
}));

function createHarness() {
  let now = 0;
  let nextFrameId = 1;
  let nextTimerId = 1;
  let visible = true;
  let focusAllowed = true;
  let connected = true;
  let rect = { width: 800, height: 400 };
  const frames = new Map<number, FrameRequestCallback>();
  const timers = new Map<number, { at: number; callback: () => void }>();
  const terminal = {
    cols: 80,
    rows: 24,
    clearTextureAtlas: vi.fn(),
    focus: vi.fn(),
    refresh: vi.fn(),
  };
  let proposal: TerminalFitDimensions = { cols: 100, rows: 30 };
  const fitAddon = {
    fit: vi.fn(() => {
      terminal.cols = proposal.cols;
      terminal.rows = proposal.rows;
    }),
    proposeDimensions: vi.fn(() => proposal),
  };
  const container = {
    get isConnected() {
      return connected;
    },
    getBoundingClientRect: () => rect,
  } as unknown as HTMLElement;
  const scheduler = new TerminalFitScheduler({
    sessionId: "session-1",
    getTerminal: () => terminal,
    getFitAddon: () => fitAddon,
    getContainer: () => container,
    isVisible: () => visible,
    canFocus: () => focusAllowed,
    requestAnimationFrame: (callback) => {
      const id = nextFrameId;
      nextFrameId += 1;
      frames.set(id, callback);
      return id;
    },
    cancelAnimationFrame: (id) => {
      frames.delete(id);
    },
    setTimeout: (callback, delay) => {
      const id = nextTimerId;
      nextTimerId += 1;
      timers.set(id, { at: now + delay, callback });
      return id;
    },
    clearTimeout: (id) => {
      timers.delete(id);
    },
    now: () => now,
  });

  const flushFrame = () => {
    const callbacks = [...frames.entries()];
    frames.clear();
    for (const [, callback] of callbacks) {
      callback(now);
    }
  };

  const advance = (ms: number) => {
    now += ms;
    const due = [...timers.entries()].filter(([, timer]) => timer.at <= now);
    for (const [id, timer] of due) {
      timers.delete(id);
      timer.callback();
    }
  };

  return {
    advance,
    fitAddon,
    flushFrame,
    scheduler,
    setConnected: (next: boolean) => {
      connected = next;
    },
    setFocusAllowed: (next: boolean) => {
      focusAllowed = next;
    },
    setProposal: (next: TerminalFitDimensions) => {
      proposal = next;
    },
    setRect: (next: typeof rect) => {
      rect = next;
    },
    setVisible: (next: boolean) => {
      visible = next;
    },
    terminal,
    timers,
  };
}

describe("TerminalFitScheduler", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("fits once for repeated identical observed sizes", () => {
    const { fitAddon, flushFrame, scheduler } = createHarness();

    scheduler.observeResize(800, 400);
    scheduler.observeResize(800, 400);
    flushFrame();

    expect(fitAddon.fit).toHaveBeenCalledTimes(1);
  });

  it("coalesces multiple requests in the same frame into the latest fit", () => {
    const { fitAddon, flushFrame, scheduler, setProposal, terminal } = createHarness();

    scheduler.schedule({ reason: "global-refresh" });
    setProposal({ cols: 120, rows: 40 });
    scheduler.schedule({ reason: "window-resized" });
    flushFrame();

    expect(fitAddon.fit).toHaveBeenCalledTimes(1);
    expect(terminal.cols).toBe(120);
    expect(terminal.rows).toBe(40);
  });

  it("skips zero-sized and hidden ordinary observer fits", () => {
    const { fitAddon, flushFrame, scheduler, setVisible } = createHarness();

    scheduler.observeResize(0, 400);
    setVisible(false);
    scheduler.observeResize(800, 400);
    flushFrame();

    expect(fitAddon.fit).not.toHaveBeenCalled();
  });

  it("skips ordinary fits when proposed geometry already matches", () => {
    const { fitAddon, flushFrame, scheduler, setProposal } = createHarness();

    setProposal({ cols: 80, rows: 24 });
    scheduler.schedule({ reason: "window-resized" });
    flushFrame();

    expect(fitAddon.fit).not.toHaveBeenCalled();
  });

  it("runs forced refresh work and texture clearing", () => {
    const { fitAddon, flushFrame, scheduler, terminal } = createHarness();

    scheduler.schedule({
      reason: "scale-factor",
      force: true,
      refresh: true,
      clearTextureAtlas: true,
      focus: true,
    });
    flushFrame();

    expect(fitAddon.fit).toHaveBeenCalledTimes(1);
    expect(terminal.clearTextureAtlas).toHaveBeenCalledTimes(1);
    expect(terminal.refresh).toHaveBeenCalledTimes(1);
    expect(terminal.focus).toHaveBeenCalledTimes(1);
  });

  it("rechecks focus permission when a queued fit executes", () => {
    const { flushFrame, scheduler, setFocusAllowed, terminal } = createHarness();

    scheduler.schedule({
      reason: "active",
      force: true,
      focus: true,
    });
    setFocusAllowed(false);
    flushFrame();

    expect(terminal.focus).not.toHaveBeenCalled();
  });

  it("does not clear texture atlas for ordinary resize", () => {
    const { flushFrame, scheduler, terminal } = createHarness();

    scheduler.observeResize(800, 400);
    flushFrame();

    expect(terminal.clearTextureAtlas).not.toHaveBeenCalled();
  });

  it("does not execute a pending RAF after disposal", () => {
    const { fitAddon, flushFrame, scheduler } = createHarness();

    scheduler.schedule({ reason: "global-refresh" });
    scheduler.dispose();
    flushFrame();

    expect(fitAddon.fit).not.toHaveBeenCalled();
  });

  it("suppresses A-B-A-B oscillation and applies a final settle fit", () => {
    const { advance, fitAddon, flushFrame, scheduler, setProposal, terminal } = createHarness();

    setProposal({ cols: 120, rows: 32 });
    scheduler.schedule({ reason: "observer" });
    flushFrame();
    setProposal({ cols: 121, rows: 32 });
    scheduler.schedule({ reason: "observer" });
    flushFrame();
    setProposal({ cols: 120, rows: 32 });
    scheduler.schedule({ reason: "observer" });
    flushFrame();
    setProposal({ cols: 121, rows: 32 });
    scheduler.schedule({ reason: "observer" });
    flushFrame();

    expect(fitAddon.fit).toHaveBeenCalledTimes(3);
    expect(terminal.cols).toBe(120);

    advance(160);
    flushFrame();

    expect(fitAddon.fit).toHaveBeenCalledTimes(4);
    expect(terminal.cols).toBe(121);
  });
});

describe("TerminalResizeDeduper", () => {
  it("sends the initial size and skips unchanged repeats", () => {
    const deduper = new TerminalResizeDeduper();

    expect(deduper.shouldSend("a", 1, 80, 24)).toBe(true);
    expect(deduper.shouldSend("a", 1, 80, 24)).toBe(false);
    expect(deduper.shouldSend("a", 1, 81, 24)).toBe(true);
  });

  it("resets when session or generation changes", () => {
    const deduper = new TerminalResizeDeduper();

    expect(deduper.shouldSend("a", 1, 80, 24)).toBe(true);
    expect(deduper.shouldSend("a", 2, 80, 24)).toBe(true);
    expect(deduper.shouldSend("b", 2, 80, 24)).toBe(true);
  });

  it("does not send invalid dimensions", () => {
    const deduper = new TerminalResizeDeduper();

    expect(deduper.shouldSend("a", 1, 0, 24)).toBe(false);
    expect(deduper.shouldSend("a", 1, 80, 0)).toBe(false);
  });
});
