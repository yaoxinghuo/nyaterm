import type {
  IBufferCell,
  IBufferLine,
  IDecoration,
  IDisposable,
  IMarker,
  Terminal,
} from "@xterm/xterm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ResolvedHighlightRule } from "./keywordHighlightPresets";
import { KeywordHighlighter } from "./keywordHighlighter";
import { XTERM_PERFORMANCE_CONFIG } from "./xtermPerformance";

vi.mock("./logger", () => ({ logger: { debug: vi.fn() } }));

class FakeDisposable implements IDisposable {
  protected disposed = false;

  dispose(): void {
    this.disposed = true;
  }
}

class FakeMarker extends FakeDisposable implements IMarker {
  readonly id = 1;
  private listeners = new Set<() => void>();

  get isDisposed(): boolean {
    return this.disposed;
  }

  constructor(public line: number) {
    super();
  }

  onDispose(listener: () => void): IDisposable {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }

  override dispose(): void {
    if (this.disposed) return;
    super.dispose();
    for (const listener of [...this.listeners]) listener();
    this.listeners.clear();
  }
}

class FakeDecoration extends FakeDisposable implements IDecoration {
  readonly options = { overviewRulerOptions: undefined };
  readonly marker = new FakeMarker(0);
  readonly element = undefined;
  private listeners = new Set<() => void>();

  get isDisposed(): boolean {
    return this.disposed;
  }

  onRender(): IDisposable {
    return new FakeDisposable();
  }

  onDispose(listener: () => void): IDisposable {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }

  override dispose(): void {
    if (this.disposed) return;
    super.dispose();
    for (const listener of [...this.listeners]) listener();
    this.listeners.clear();
  }
}

interface CellSpec {
  chars: string;
  width: number;
  fgDefault?: boolean;
}

function createCell(spec: CellSpec = { chars: "", width: 1 }): IBufferCell {
  return {
    getChars: () => spec.chars,
    getWidth: () => spec.width,
    isFgDefault: () => spec.fgDefault ?? true,
  } as unknown as IBufferCell;
}

interface FakeLine extends IBufferLine {
  translateSpy: ReturnType<typeof vi.fn>;
}

function createLine(
  text: string,
  options: { wrapped?: boolean; cells?: Map<number, CellSpec>; fgDefault?: boolean } = {},
): FakeLine {
  const translateSpy = vi.fn((trimRight: boolean) =>
    trimRight ? text.replace(/\s+$/u, "") : text,
  );
  return {
    isWrapped: options.wrapped ?? false,
    length: Math.max(80, text.length),
    translateSpy,
    translateToString: translateSpy,
    getCell: (index: number) => {
      const explicit = options.cells?.get(index);
      if (explicit) return createCell(explicit);
      const char = text[index] ?? "";
      return createCell({ chars: char, width: 1, fgDefault: options.fgDefault });
    },
  } as unknown as FakeLine;
}

interface DecorationRecord {
  decoration: FakeDecoration;
  marker: FakeMarker;
  x?: number;
  width?: number;
  foregroundColor?: string;
}

function createHarness(options: {
  lines: FakeLine[];
  baseY: number;
  viewportY: number;
  rows?: number;
  cols?: number;
}) {
  const writeListeners = new Set<() => void>();
  const resizeListeners = new Set<() => void>();
  const renderListeners = new Set<() => void>();
  const markers: FakeMarker[] = [];
  const decorations: DecorationRecord[] = [];
  const getLineSpy = vi.fn((lineY: number) => options.lines[lineY]);
  const active = {
    type: "normal" as "normal" | "alternate",
    baseY: options.baseY,
    cursorY: 0,
    viewportY: options.viewportY,
    get length() {
      return options.lines.length;
    },
    getLine: getLineSpy,
    getNullCell: () => createCell(),
  };
  const subscribe = (listeners: Set<() => void>, listener: () => void): IDisposable => {
    listeners.add(listener);
    return { dispose: () => listeners.delete(listener) };
  };
  const terminal = {
    rows: options.rows ?? 1,
    cols: options.cols ?? 80,
    buffer: { active },
    onWriteParsed: (listener: () => void) => subscribe(writeListeners, listener),
    onResize: (listener: () => void) => subscribe(resizeListeners, listener),
    onRender: (listener: () => void) => subscribe(renderListeners, listener),
    registerMarker: (offset = 0) => {
      const marker = new FakeMarker(active.baseY + active.cursorY + offset);
      markers.push(marker);
      return marker;
    },
    registerDecoration: (decorationOptions: {
      marker: FakeMarker;
      x?: number;
      width?: number;
      foregroundColor?: string;
    }) => {
      const decoration = new FakeDecoration();
      decorations.push({ decoration, ...decorationOptions });
      return decoration;
    },
  } as unknown as Terminal;

  return {
    active,
    decorations,
    getLineSpy,
    markers,
    terminal,
    render: () => {
      for (const listener of renderListeners) listener();
    },
    resize: () => {
      for (const listener of resizeListeners) listener();
    },
    write: () => {
      for (const listener of writeListeners) listener();
    },
  };
}

const rule = (pattern = "ERROR", color = "#ff0000"): ResolvedHighlightRule => ({
  id: `rule-${pattern}-${color}`,
  name: pattern,
  patterns: [pattern],
  color,
  enabled: true,
});

function flushWriteRefresh() {
  vi.advanceTimersByTime(XTERM_PERFORMANCE_CONFIG.highlighting.writeRefreshIntervalMs);
}

function flushScrollRefresh() {
  vi.advanceTimersByTime(XTERM_PERFORMANCE_CONFIG.highlighting.scrollIdleDebounceMs);
}

describe("KeywordHighlighter", () => {
  let rafCallbacks: Map<number, FrameRequestCallback>;
  let nextRafId: number;

  beforeEach(() => {
    vi.useFakeTimers();
    rafCallbacks = new Map();
    nextRafId = 1;
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      const id = nextRafId++;
      rafCallbacks.set(id, callback);
      return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => rafCallbacks.delete(id));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("refreshes keyword highlights during continuous writes", () => {
    const lines = Array.from({ length: 40 }, () => createLine(""));
    const harness = createHarness({ lines, baseY: 20, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    lines[20] = createLine("ERROR");

    const interval = XTERM_PERFORMANCE_CONFIG.highlighting.writeRefreshIntervalMs;
    for (let refresh = 1; refresh <= 3; refresh++) {
      for (let write = 0; write < 4; write++) {
        harness.write();
        expect(lines[20].translateSpy).toHaveBeenCalledTimes(refresh - 1);
        vi.advanceTimersByTime(interval / 4);
      }
      expect(lines[20].translateSpy).toHaveBeenCalledTimes(refresh);
    }

    expect(harness.decorations.filter((entry) => entry.marker.line === 20)).toHaveLength(1);
    highlighter.dispose();
  });

  it("coalesces burst writes into one delayed highlight refresh", () => {
    const lines = Array.from({ length: 40 }, () => createLine(""));
    const harness = createHarness({ lines, baseY: 20, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    lines[20] = createLine("ERROR");

    const interval = XTERM_PERFORMANCE_CONFIG.highlighting.writeRefreshIntervalMs;
    for (let write = 0; write < 10; write++) {
      harness.write();
      if (write < 9) vi.advanceTimersByTime(interval / 10);
    }
    vi.advanceTimersByTime(interval / 10 - 1);
    expect(lines[20].translateSpy).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(lines[20].translateSpy).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(interval * 2);
    expect(lines[20].translateSpy).toHaveBeenCalledTimes(1);
    highlighter.dispose();
  });

  it("uses a pure trailing debounce for continuous scrolling", () => {
    const lines = Array.from({ length: 240 }, (_, index) =>
      createLine(index === 140 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 220, viewportY: 100 });
    const highlighter = new KeywordHighlighter(harness.terminal, "session-1");
    highlighter.setRules([rule()], true);
    flushWriteRefresh();

    harness.active.viewportY = 120;
    harness.render();
    vi.advanceTimersByTime(60);
    harness.active.viewportY = 130;
    harness.render();
    vi.advanceTimersByTime(119);
    expect(lines[140].translateSpy).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(lines[140].translateSpy).toHaveBeenCalledTimes(1);
    highlighter.dispose();
  });

  it("lets scrolling cancel a pending continuation frame", () => {
    const denseText = `${"ERROR ".repeat(20)}`;
    const lines = Array.from({ length: 160 }, (_, index) =>
      createLine(index >= 20 && index <= 60 ? denseText : ""),
    );
    const harness = createHarness({ lines, baseY: 140, viewportY: 40 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    expect(rafCallbacks).toHaveLength(1);
    const decorationCount = harness.decorations.length;

    harness.active.viewportY = 41;
    harness.render();
    expect(rafCallbacks).toHaveLength(0);
    expect(harness.decorations).toHaveLength(decorationCount);
    highlighter.dispose();
  });

  it("rebuilds decorations from cached scrollback spans without rescanning", () => {
    const lines = Array.from({ length: 240 }, (_, index) =>
      createLine(index === 100 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 220, viewportY: 100 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    expect(lines[100].translateSpy).toHaveBeenCalledTimes(1);

    harness.active.viewportY = 160;
    harness.render();
    flushScrollRefresh();
    const firstDecoration = harness.decorations.find((entry) => entry.marker.line === 100);
    expect(firstDecoration?.decoration).toHaveProperty("disposed", true);

    harness.active.viewportY = 100;
    harness.render();
    flushScrollRefresh();
    expect(lines[100].translateSpy).toHaveBeenCalledTimes(1);
    expect(harness.decorations.filter((entry) => entry.marker.line === 100)).toHaveLength(2);
    highlighter.dispose();
  });

  it("caches empty immutable line results", () => {
    const lines = Array.from({ length: 240 }, () => createLine("nothing"));
    const harness = createHarness({ lines, baseY: 220, viewportY: 100 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();

    harness.active.viewportY = 160;
    harness.render();
    flushScrollRefresh();
    harness.active.viewportY = 100;
    harness.render();
    flushScrollRefresh();
    expect(lines[100].translateSpy).toHaveBeenCalledTimes(1);
    highlighter.dispose();
  });

  it("keeps the live screen dynamic instead of storing it in the match cache", () => {
    const lines = Array.from({ length: 40 }, (_, index) =>
      createLine(index === 20 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 20, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    harness.write();
    flushWriteRefresh();

    expect(lines[20].translateSpy).toHaveBeenCalledTimes(2);
    highlighter.dispose();
  });

  it("invalidates decorations and match state in the alternate screen", () => {
    const lines = Array.from({ length: 80 }, (_, index) =>
      createLine(index === 20 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 60, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    const firstDecoration = harness.decorations.find((entry) => entry.marker.line === 20);

    harness.active.type = "alternate";
    harness.write();
    expect(firstDecoration?.decoration).toHaveProperty("disposed", true);
    const internals = highlighter as unknown as { lineMatchCache: Map<number, unknown> };
    expect(internals.lineMatchCache).toHaveLength(0);

    harness.active.type = "normal";
    harness.write();
    flushWriteRefresh();
    expect(lines[20].translateSpy).toHaveBeenCalledTimes(2);
    highlighter.dispose();
  });

  it("bounds the match cache with LRU eviction", () => {
    const harness = createHarness({ lines: [createLine("")], baseY: 0, viewportY: 0 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    const internals = highlighter as unknown as {
      setCachedMatches: (lineY: number, spans: []) => void;
      lineMatchCache: Map<number, unknown>;
    };
    for (let lineY = 0; lineY <= XTERM_PERFORMANCE_CONFIG.highlighting.maxCachedMatchLines; lineY++) {
      internals.setCachedMatches(lineY, []);
    }
    expect(internals.lineMatchCache).toHaveLength(
      XTERM_PERFORMANCE_CONFIG.highlighting.maxCachedMatchLines,
    );
    expect(internals.lineMatchCache.has(0)).toBe(false);
    expect(internals.lineMatchCache.has(1)).toBe(true);
    highlighter.dispose();
  });

  it("invalidates match and decoration caches after scrollback trim and resize", () => {
    const lines = Array.from({ length: 80 }, (_, index) =>
      createLine(index === 20 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 60, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    const firstSentinel = harness.markers[0];
    firstSentinel.dispose();
    harness.active.viewportY = 21;
    harness.render();
    flushScrollRefresh();
    expect(lines[20].translateSpy).toHaveBeenCalledTimes(2);

    harness.resize();
    flushWriteRefresh();
    expect(lines[20].translateSpy).toHaveBeenCalledTimes(3);
    highlighter.dispose();
  });

  it("invalidates cached colors when rules change and preserves priority/overlap", () => {
    const lines = Array.from({ length: 80 }, (_, index) =>
      createLine(index === 20 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 60, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule("ERROR", "#111111"), rule("ERR", "#222222")], true);
    flushWriteRefresh();
    expect(
      harness.decorations.filter((entry) => entry.marker.line === 20).map((entry) => entry.foregroundColor),
    ).toEqual(["#111111"]);

    highlighter.setRules([rule("ERROR", "#333333")], true);
    flushWriteRefresh();
    expect(lines[20].translateSpy).toHaveBeenCalledTimes(2);
    expect(harness.decorations[harness.decorations.length - 1]?.foregroundColor).toBe("#333333");
    highlighter.dispose();
  });

  it("does not override an ANSI foreground", () => {
    const lines = Array.from({ length: 80 }, (_, index) =>
      createLine(index === 20 ? "ERROR" : "", { fgDefault: index !== 20 }),
    );
    const harness = createHarness({ lines, baseY: 60, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    flushWriteRefresh();
    expect(harness.decorations.filter((entry) => entry.marker.line === 20)).toHaveLength(0);
    highlighter.dispose();
  });

  it("maps CJK and surrogate-pair string offsets to xterm cells", () => {
    const cjkCells = new Map<number, CellSpec>();
    [..."错误：连接失败"].forEach((char, index) => {
      cjkCells.set(index * 2, { chars: char, width: 2 });
    });
    const emojiCells = new Map<number, CellSpec>([
      [0, { chars: "🙂", width: 2 }],
      [2, { chars: "E", width: 1 }],
      [3, { chars: "R", width: 1 }],
      [4, { chars: "R", width: 1 }],
      [5, { chars: "O", width: 1 }],
      [6, { chars: "R", width: 1 }],
    ]);
    const nulBeforeWideCells = new Map<number, CellSpec>([
      [0, { chars: "", width: 0 }],
      [1, { chars: "错", width: 2 }],
      [3, { chars: "误", width: 2 }],
    ]);
    const lines = Array.from({ length: 80 }, () => createLine(""));
    lines[20] = createLine("错误：连接失败", { cells: cjkCells });
    lines[21] = createLine("🙂ERROR", { cells: emojiCells });
    lines[22] = createLine(" 错误", { cells: nulBeforeWideCells });
    const harness = createHarness({ lines, baseY: 60, viewportY: 20, rows: 3 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules(
      [rule("连接失败"), rule("ERROR", "#00ff00"), rule("错误", "#0000ff")],
      true,
    );
    flushWriteRefresh();

    expect(harness.decorations).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ marker: expect.objectContaining({ line: 20 }), x: 6, width: 8 }),
        expect.objectContaining({ marker: expect.objectContaining({ line: 21 }), x: 2, width: 5 }),
        expect.objectContaining({ marker: expect.objectContaining({ line: 22 }), x: 1, width: 4 }),
      ]),
    );
    highlighter.dispose();
  });

  it("caches complete wrapped logical lines and restores both physical rows", () => {
    const lines = Array.from({ length: 240 }, () => createLine(""));
    lines[100] = createLine("ERR");
    lines[101] = createLine("OR", { wrapped: true });
    const harness = createHarness({ lines, baseY: 220, viewportY: 100, rows: 2, cols: 3 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true, true);
    flushWriteRefresh();
    expect(lines[100].translateSpy).toHaveBeenCalledTimes(1);
    expect(lines[101].translateSpy).toHaveBeenCalledTimes(1);
    expect(harness.decorations).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ marker: expect.objectContaining({ line: 100 }), width: 3 }),
        expect.objectContaining({ marker: expect.objectContaining({ line: 101 }), width: 2 }),
      ]),
    );

    harness.active.viewportY = 160;
    harness.render();
    flushScrollRefresh();
    harness.active.viewportY = 100;
    harness.render();
    flushScrollRefresh();
    expect(lines[100].translateSpy).toHaveBeenCalledTimes(1);
    expect(lines[101].translateSpy).toHaveBeenCalledTimes(1);
    highlighter.dispose();
  });

  it.each([false, true])(
    "skips a decoration-dense wrapped logical line atomically when cross-wrap matching is %s",
    (highlightAcrossWrappedLines) => {
      const denseText = "ERROR ".repeat(XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine);
      const wrappedRows =
        Math.floor(
          XTERM_PERFORMANCE_CONFIG.highlighting.maxDecorationsPerLogicalLine /
            XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine,
        ) + 1;
      const startY = 100;
      const normalY = startY + wrappedRows;
      const lines = Array.from({ length: 240 }, () => createLine(""));
      for (let index = 0; index < wrappedRows; index++) {
        lines[startY + index] = createLine(denseText, { wrapped: index > 0 });
      }
      lines[normalY] = createLine("ERROR");
      const harness = createHarness({
        lines,
        baseY: 220,
        viewportY: startY,
        rows: wrappedRows + 1,
        cols: denseText.length,
      });
      const highlighter = new KeywordHighlighter(harness.terminal);
      highlighter.setRules([rule()], true, highlightAcrossWrappedLines);
      flushWriteRefresh();

      expect(
        harness.decorations.filter(
          (entry) => entry.marker.line >= startY && entry.marker.line < normalY,
        ),
      ).toHaveLength(0);
      expect(harness.decorations.filter((entry) => entry.marker.line === normalY)).toHaveLength(1);
      expect(rafCallbacks).toHaveLength(0);
      for (let lineY = startY; lineY < normalY; lineY++) {
        expect(lines[lineY].translateSpy).toHaveBeenCalledTimes(1);
      }

      harness.write();
      flushWriteRefresh();
      expect(rafCallbacks).toHaveLength(0);
      for (let lineY = startY; lineY < normalY; lineY++) {
        expect(lines[lineY].translateSpy).toHaveBeenCalledTimes(1);
      }

      harness.active.viewportY = 160;
      harness.render();
      flushScrollRefresh();
      harness.active.viewportY = startY;
      harness.render();
      flushScrollRefresh();
      expect(
        harness.decorations.filter(
          (entry) => entry.marker.line >= startY && entry.marker.line < normalY,
        ),
      ).toHaveLength(0);
      for (let lineY = startY; lineY < normalY; lineY++) {
        expect(lines[lineY].translateSpy).toHaveBeenCalledTimes(1);
      }
      highlighter.dispose();
    },
  );

  it.each([false, true])(
    "drops existing wrapped-line decorations atomically after the logical-line cap is exceeded (%s)",
    (highlightAcrossWrappedLines) => {
      const denseText = "ERROR ".repeat(XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine);
      const denseRows = Math.floor(
        XTERM_PERFORMANCE_CONFIG.highlighting.maxDecorationsPerLogicalLine /
          XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine,
      );
      const startY = 100;
      const lines = Array.from({ length: 140 }, () => createLine(""));
      for (let index = 0; index < denseRows; index++) {
        lines[startY + index] = createLine(denseText, { wrapped: index > 0 });
      }
      lines[startY + denseRows] = createLine("", { wrapped: true });
      const harness = createHarness({
        lines,
        baseY: startY,
        viewportY: startY,
        rows: denseRows + 1,
        cols: denseText.length,
      });
      const highlighter = new KeywordHighlighter(harness.terminal);
      highlighter.setRules([rule()], true, highlightAcrossWrappedLines);
      flushWriteRefresh();

      const initialDecorations = harness.decorations.filter(
        (entry) => entry.marker.line >= startY && entry.marker.line <= startY + denseRows,
      );
      expect(initialDecorations).toHaveLength(
        XTERM_PERFORMANCE_CONFIG.highlighting.maxDecorationsPerLogicalLine,
      );
      expect(rafCallbacks).toHaveLength(0);

      lines[startY + denseRows] = createLine(denseText, { wrapped: true });
      harness.write();
      flushWriteRefresh();

      expect(harness.decorations).toHaveLength(initialDecorations.length);
      expect(initialDecorations.every((entry) => entry.decoration.isDisposed)).toBe(true);
      expect(rafCallbacks).toHaveLength(0);
      highlighter.dispose();
    },
  );

  it.each([false, true])(
    "skips a wrapped logical line after its refresh budget expires and continues with later lines (%s)",
    (highlightAcrossWrappedLines) => {
      const denseText = "ERROR ".repeat(XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine);
      const wrappedRows =
        Math.floor(
          XTERM_PERFORMANCE_CONFIG.highlighting.maxDecorationsPerLogicalLine /
            XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine,
        ) + 1;
      const startY = 100;
      const normalY = startY + wrappedRows;
      let now = 0;
      const lines = Array.from({ length: normalY + 1 }, () => createLine(""));
      for (let index = 0; index < wrappedRows; index++) {
        const line = createLine(denseText, { wrapped: index > 0 });
        line.translateSpy.mockImplementation((trimRight: boolean) => {
          now = 4;
          return trimRight ? denseText.replace(/\s+$/u, "") : denseText;
        });
        lines[startY + index] = line;
      }
      lines[normalY] = createLine("ERROR");
      const harness = createHarness({
        lines,
        baseY: startY,
        viewportY: startY,
        rows: wrappedRows + 1,
        cols: denseText.length,
      });
      const highlighter = new KeywordHighlighter(harness.terminal);
      const performanceNow = vi.spyOn(performance, "now").mockImplementation(() => now);

      highlighter.setRules([rule()], true, highlightAcrossWrappedLines);
      flushWriteRefresh();
      expect(rafCallbacks).toHaveLength(1);
      expect(harness.decorations).toHaveLength(0);
      const initialScanCounts = Array.from(
        { length: wrappedRows },
        (_, index) => lines[startY + index].translateSpy.mock.calls.length,
      );

      const [[rafId, continuation]] = [...rafCallbacks.entries()];
      rafCallbacks.delete(rafId);
      now = 100;
      continuation(100);

      expect(rafCallbacks).toHaveLength(0);
      expect(
        Array.from(
          { length: wrappedRows },
          (_, index) => lines[startY + index].translateSpy.mock.calls.length,
        ),
      ).toEqual(initialScanCounts);
      expect(
        harness.decorations.filter(
          (entry) => entry.marker.line >= startY && entry.marker.line < normalY,
        ),
      ).toHaveLength(0);
      expect(harness.decorations.filter((entry) => entry.marker.line === normalY)).toHaveLength(1);
      performanceNow.mockRestore();
      highlighter.dispose();
    },
  );

  it.each([false, true])(
    "skips a wrapped logical line when the refresh budget expires after its final scan check (%s)",
    (highlightAcrossWrappedLines) => {
      const startY = 100;
      const normalY = startY + 2;
      const lines = Array.from({ length: normalY + 1 }, () => createLine(""));
      lines[startY] = createLine("ERROR");
      lines[startY + 1] = createLine("ERROR", { wrapped: true });
      lines[normalY] = createLine("ERROR");
      const harness = createHarness({
        lines,
        baseY: startY,
        viewportY: startY,
        rows: 3,
      });
      const highlighter = new KeywordHighlighter(harness.terminal);
      let now = 0;
      let errorNullMatches = 0;
      const originalExec = RegExp.prototype.exec;
      const execSpy = vi.spyOn(RegExp.prototype, "exec").mockImplementation(function (this: RegExp, text: string) {
        const result = originalExec.call(this, text);
        if (this.source === "ERROR" && result === null) {
          errorNullMatches++;
          const expiryNullMatch = highlightAcrossWrappedLines ? 1 : 2;
          if (errorNullMatches === expiryNullMatch) now = 4;
        }
        return result;
      });
      const performanceNow = vi.spyOn(performance, "now").mockImplementation(() => now);

      highlighter.setRules([rule()], true, highlightAcrossWrappedLines);
      flushWriteRefresh();
      expect(rafCallbacks).toHaveLength(1);
      expect(harness.decorations).toHaveLength(0);
      const initialScanCounts = [
        lines[startY].translateSpy.mock.calls.length,
        lines[startY + 1].translateSpy.mock.calls.length,
      ];

      const [[rafId, continuation]] = [...rafCallbacks.entries()];
      rafCallbacks.delete(rafId);
      now = 100;
      continuation(100);

      expect(rafCallbacks).toHaveLength(0);
      expect([
        lines[startY].translateSpy.mock.calls.length,
        lines[startY + 1].translateSpy.mock.calls.length,
      ]).toEqual(initialScanCounts);
      expect(
        harness.decorations.filter(
          (entry) => entry.marker.line === startY || entry.marker.line === startY + 1,
        ),
      ).toHaveLength(0);
      expect(harness.decorations.filter((entry) => entry.marker.line === normalY)).toHaveLength(1);
      execSpy.mockRestore();
      performanceNow.mockRestore();
      highlighter.dispose();
    },
  );

  it.each([false, true])(
    "bounds a 10,000-row logical line and reuses immutable suppression (%s)",
    (highlightAcrossWrappedLines) => {
      const giantRows = 10_000;
      const normalY = giantRows;
      const lines = Array.from({ length: giantRows + 1 }, (_, index) =>
        createLine(index === normalY ? "ERROR" : "", {
          wrapped: index > 0 && index < normalY,
        }),
      );
      const rows = 501;
      const viewportY = giantRows - 500;
      const harness = createHarness({ lines, baseY: normalY, viewportY, rows });
      const highlighter = new KeywordHighlighter(harness.terminal);
      let now = 0;
      const performanceNow = vi.spyOn(performance, "now").mockImplementation(() => now);

      highlighter.setRules([rule()], true, highlightAcrossWrappedLines);
      flushWriteRefresh();

      const maxBoundedReads =
        XTERM_PERFORMANCE_CONFIG.highlighting.maxLogicalLineRows * 2 + 100;
      expect(harness.getLineSpy.mock.calls.length).toBeLessThan(maxBoundedReads);
      expect(lines[giantRows - 1].translateSpy).not.toHaveBeenCalled();
      expect(harness.decorations.filter((entry) => entry.marker.line === normalY)).toHaveLength(1);
      expect(rafCallbacks).toHaveLength(0);

      const internals = highlighter as unknown as {
        suppressedLineCache: Map<number, true>;
      };
      expect(internals.suppressedLineCache.size).toBeGreaterThan(0);
      const firstRefreshReads = harness.getLineSpy.mock.calls.length;

      harness.active.viewportY--;
      harness.render();
      flushScrollRefresh();

      const scrollRefreshReads = harness.getLineSpy.mock.calls.length - firstRefreshReads;
      expect(scrollRefreshReads).toBeLessThan(rows + 50);
      const readsBeforeWrite = harness.getLineSpy.mock.calls.length;

      harness.getLineSpy.mockImplementation((lineY: number) => {
        now++;
        return lines[lineY];
      });
      now = 100;
      harness.write();
      flushWriteRefresh();

      const secondRefreshReads = harness.getLineSpy.mock.calls.length - readsBeforeWrite;
      expect(secondRefreshReads).toBeLessThan(rows + 50);
      expect(lines[giantRows - 1].translateSpy).not.toHaveBeenCalled();

      harness.resize();
      expect(internals.suppressedLineCache).toHaveLength(0);
      performanceNow.mockRestore();
      highlighter.dispose();
    },
  );

  it.each([false, true])(
    "keeps a logical-line boundary timeout out of the persistent suppression cache (%s)",
    (highlightAcrossWrappedLines) => {
      const lines = [
        createLine("ERROR"),
        createLine("ERROR", { wrapped: true }),
        createLine("ERROR"),
      ];
      const harness = createHarness({ lines, baseY: 2, viewportY: 0, rows: 3 });
      let now = 0;
      let lineReads = 0;
      harness.getLineSpy.mockImplementation((lineY: number) => {
        lineReads++;
        if (lineReads === 2) now = 4;
        return lines[lineY];
      });
      const performanceNow = vi.spyOn(performance, "now").mockImplementation(() => now);
      const highlighter = new KeywordHighlighter(harness.terminal);

      highlighter.setRules([rule()], true, highlightAcrossWrappedLines);
      flushWriteRefresh();

      const internals = highlighter as unknown as {
        suppressedLineCache: Map<number, true>;
      };
      expect(internals.suppressedLineCache).toHaveLength(0);
      expect(lines[0].translateSpy).not.toHaveBeenCalled();
      expect(lines[1].translateSpy).not.toHaveBeenCalled();
      expect(rafCallbacks).toHaveLength(1);

      const [[rafId, continuation]] = [...rafCallbacks.entries()];
      rafCallbacks.delete(rafId);
      now = 100;
      continuation(100);

      expect(lines[0].translateSpy).not.toHaveBeenCalled();
      expect(lines[1].translateSpy).not.toHaveBeenCalled();
      expect(harness.decorations.filter((entry) => entry.marker.line === 2)).toHaveLength(1);
      expect(rafCallbacks).toHaveLength(0);

      now = 200;
      harness.write();
      flushWriteRefresh();

      expect(lines[0].translateSpy).toHaveBeenCalledTimes(1);
      expect(lines[1].translateSpy).toHaveBeenCalledTimes(1);
      expect(internals.suppressedLineCache).toHaveLength(0);
      performanceNow.mockRestore();
      highlighter.dispose();
    },
  );

  it("only persists row-limit suppression for immutable scrollback rows", () => {
    const logicalRows = 600;
    const lines = Array.from({ length: logicalRows + 1 }, (_, index) =>
      createLine(index === logicalRows ? "ERROR" : "", {
        wrapped: index > 0 && index < logicalRows,
      }),
    );
    const harness = createHarness({ lines, baseY: 550, viewportY: 540, rows: 30 });
    const highlighter = new KeywordHighlighter(harness.terminal);

    highlighter.setRules([rule()], true);
    flushWriteRefresh();

    const internals = highlighter as unknown as {
      suppressedLineCache: Map<number, true>;
    };
    expect(internals.suppressedLineCache.size).toBeGreaterThan(0);
    expect([...internals.suppressedLineCache.keys()].every((lineY) => lineY < harness.active.baseY)).toBe(
      true,
    );
    highlighter.dispose();
  });

  it("invalidates deterministic suppression after scrollback trimming", () => {
    const denseText = "ERROR ".repeat(XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine);
    const wrappedRows =
      Math.floor(
        XTERM_PERFORMANCE_CONFIG.highlighting.maxDecorationsPerLogicalLine /
          XTERM_PERFORMANCE_CONFIG.highlighting.maxMatchesPerLine,
      ) + 1;
    const startY = 100;
    const lines = Array.from({ length: 240 }, () => createLine(""));
    for (let index = 0; index < wrappedRows; index++) {
      lines[startY + index] = createLine(denseText, { wrapped: index > 0 });
    }
    const harness = createHarness({
      lines,
      baseY: 220,
      viewportY: startY,
      rows: wrappedRows,
      cols: denseText.length,
    });
    const highlighter = new KeywordHighlighter(harness.terminal);

    highlighter.setRules([rule()], true);
    flushWriteRefresh();

    const internals = highlighter as unknown as {
      suppressedLineCache: Map<number, true>;
    };
    expect(internals.suppressedLineCache.size).toBe(wrappedRows);

    for (let index = 0; index < wrappedRows; index++) {
      lines[startY + index] = createLine("");
    }
    harness.markers[0].dispose();
    harness.active.viewportY++;
    harness.render();
    flushScrollRefresh();

    expect(internals.suppressedLineCache).toHaveLength(0);
    highlighter.dispose();
  });

  it("delays resume and cancels it when suspension returns", () => {
    const lines = Array.from({ length: 80 }, (_, index) =>
      createLine(index === 20 ? "ERROR" : ""),
    );
    const harness = createHarness({ lines, baseY: 60, viewportY: 20 });
    const highlighter = new KeywordHighlighter(harness.terminal);
    highlighter.setRules([rule()], true);
    highlighter.setSuspended(true);
    highlighter.setSuspended(false);
    vi.advanceTimersByTime(XTERM_PERFORMANCE_CONFIG.highlighting.resumeIdleDelayMs - 1);
    expect(rafCallbacks).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(rafCallbacks).toHaveLength(1);

    highlighter.setSuspended(true);
    expect(rafCallbacks).toHaveLength(0);
    for (const callback of rafCallbacks.values()) callback(performance.now());
    expect(lines[20].translateSpy).not.toHaveBeenCalled();
    highlighter.dispose();
  });
});
