import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { Annotation, Compartment, EditorState, Transaction } from "@codemirror/state";
import {
  EditorView,
  placeholder as editorPlaceholder,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { useEffect, useLayoutEffect, useRef } from "react";
import { cn } from "@/lib/utils";

const externalChange = Annotation.define<boolean>();

const theme = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "transparent",
    color: "var(--foreground)",
    fontSize: "0.875rem",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.25rem",
    overflow: "auto",
  },
  ".cm-content": {
    padding: "8px 0",
    caretColor: "var(--foreground)",
    // JetBrains Mono uses contextual ligatures for -- and ---.
    fontVariantLigatures: "none",
    fontFeatureSettings: '"liga" 0, "calt" 0',
  },
  ".cm-line": { padding: "0 12px" },
  ".cm-gutters": {
    backgroundColor: "color-mix(in srgb, var(--muted) 40%, transparent)",
    color: "color-mix(in srgb, var(--muted-foreground) 70%, transparent)",
    borderRight: "1px solid color-mix(in srgb, var(--border) 40%, transparent)",
  },
  ".cm-lineNumbers .cm-gutterElement": {
    minWidth: "2.5rem",
    padding: "0 8px 0 4px",
    fontSize: "0.6875rem",
  },
  ".cm-placeholder": { color: "var(--muted-foreground)" },
  ".cm-content ::selection, .cm-content::selection": {
    backgroundColor: "var(--df-terminal-selection, var(--primary))",
  },
});

interface QuickCommandEditorProps {
  value: string;
  placeholder: string;
  invalid?: boolean;
  onChange: (value: string) => void;
}

export default function QuickCommandEditor({
  value,
  placeholder,
  invalid,
  onChange,
}: QuickCommandEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const initialValueRef = useRef(value);
  const onChangeRef = useRef(onChange);
  const fieldConfig = useRef(new Compartment());

  useLayoutEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    if (!hostRef.current) return;

    const view = new EditorView({
      parent: hostRef.current,
      state: EditorState.create({
        doc: initialValueRef.current,
        extensions: [
          lineNumbers(),
          EditorView.lineWrapping,
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          theme,
          fieldConfig.current.of([]),
          EditorView.updateListener.of((update) => {
            if (
              update.docChanged &&
              !update.transactions.some((transaction) => transaction.annotation(externalChange))
            ) {
              onChangeRef.current(update.state.doc.toString());
            }
          }),
        ],
      }),
    });
    viewRef.current = view;

    return () => {
      viewRef.current = null;
      view.destroy();
    };
  }, []);

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: fieldConfig.current.reconfigure([
        editorPlaceholder(placeholder),
        EditorView.contentAttributes.of({
          id: "qc-command",
          "aria-labelledby": "qc-command-label",
          "aria-multiline": "true",
          "aria-invalid": invalid ? "true" : "false",
          spellcheck: "false",
          autocapitalize: "none",
          autocorrect: "off",
          autocomplete: "off",
        }),
      ]),
    });
  }, [placeholder, invalid]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === value) return;

    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: value },
      annotations: [externalChange.of(true), Transaction.addToHistory.of(false)],
    });
  }, [value]);

  return (
    <div
      className={cn(
        "relative min-h-28 min-w-0 flex-1 overflow-hidden rounded-md border border-input bg-muted/30 text-sm shadow-xs transition-[color,box-shadow]",
        "focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50",
        invalid && "border-destructive focus-within:ring-destructive",
      )}
    >
      <div ref={hostRef} className="absolute inset-0" />
    </div>
  );
}
