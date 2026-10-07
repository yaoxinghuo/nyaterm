import { redo, undo } from "@codemirror/commands";
import { EditorView } from "@codemirror/view";
import { act, render, screen } from "@testing-library/react";
import { StrictMode } from "react";
import { describe, expect, it, vi } from "vitest";
import QuickCommandEditor from "./QuickCommandEditor";

function getView() {
  const view = EditorView.findFromDOM(screen.getByRole("textbox"));
  if (!view) throw new Error("Editor view was not mounted");
  return view;
}

describe("QuickCommandEditor", () => {
  it("preserves selection and undo history when React updates the value and field options", () => {
    const firstChange = vi.fn();
    const { rerender } = render(
      <QuickCommandEditor value="echo" placeholder="Command" onChange={firstChange} />,
    );
    const view = getView();
    act(() => {
      view.dispatch({ changes: { from: 4, insert: " hello\nexit" }, selection: { anchor: 9 } });
    });
    expect(view.state.selection.main.anchor).toBe(9);
    expect(firstChange).toHaveBeenCalledExactlyOnceWith("echo hello\nexit");

    const nextChange = vi.fn();
    rerender(
      <QuickCommandEditor
        value={"echo hello\nexit"}
        placeholder="Updated placeholder"
        invalid
        onChange={nextChange}
      />,
    );
    expect(getView()).toBe(view);
    expect(view.state.selection.main.anchor).toBe(9);
    expect(screen.getByRole("textbox").getAttribute("aria-invalid")).toBe("true");
    expect(nextChange).not.toHaveBeenCalled();

    act(() => {
      undo(view);
    });
    expect(view.state.doc.toString()).toBe("echo");
    expect(nextChange).toHaveBeenLastCalledWith("echo");
    act(() => {
      redo(view);
    });
    expect(view.state.doc.toString()).toBe("echo hello\nexit");
    expect(nextChange).toHaveBeenLastCalledWith("echo hello\nexit");
  });

  it("accepts external value changes without emitting them back as user edits", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <QuickCommandEditor value="echo initial" placeholder="Command" onChange={onChange} />,
    );
    const view = getView();
    rerender(<QuickCommandEditor value="" placeholder="New command" onChange={onChange} />);
    expect(view.state.doc.toString()).toBe("");
    expect(screen.getByText("New command")).toBeDefined();
    expect(onChange).not.toHaveBeenCalled();
    expect(undo(view)).toBe(false);
  });

  it("keeps one editor in StrictMode and removes it when unmounted", () => {
    const onChange = vi.fn();
    const { container, unmount } = render(
      <StrictMode>
        <QuickCommandEditor value="" placeholder="Command" onChange={onChange} />
      </StrictMode>,
    );
    expect(container.querySelectorAll(".cm-editor")).toHaveLength(1);
    act(() => {
      getView().dispatch({ changes: { from: 0, insert: "pwd" } });
    });
    expect(onChange).toHaveBeenCalledExactlyOnceWith("pwd");
    const editor = container.querySelector(".cm-editor");
    unmount();
    expect(editor?.isConnected).toBe(false);
  });
});
