import {
  EditorState,
  type Extension,
  StateEffect,
  StateField,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import CodeMirror from "@uiw/react-codemirror";
import { Box, useTheme } from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import type { LanguageSupport } from "@codemirror/language";
import { cmTheme } from "../../cmTheme";
import { loadCodeLanguage } from "./codeLanguage";
import { changedWordRange } from "./diffWordModel";
import { diffContextFolds } from "./diffContextModel";

function diffDecorations(view: EditorView): ReturnType<typeof Decoration.set> {
  const decorations = [];
  for (const range of view.visibleRanges) {
    let position = view.state.doc.lineAt(range.from).from;
    while (position <= range.to) {
      const line = view.state.doc.lineAt(position);
      const first = line.text[0];
      const className = first === "+" && !line.text.startsWith("+++")
        ? "cowboy-diff-added"
        : first === "-" && !line.text.startsWith("---")
        ? "cowboy-diff-removed"
        : line.text.startsWith("@@")
        ? "cowboy-diff-hunk"
        : "";
      if (className) {
        decorations.push(
          Decoration.line({ class: className }).range(line.from),
        );
      }
      if (
        first === "-" &&
        !line.text.startsWith("---") &&
        line.number < view.state.doc.lines
      ) {
        const next = view.state.doc.line(line.number + 1);
        if (next.text.startsWith("+") && !next.text.startsWith("+++")) {
          const changed = changedWordRange(
            line.text.slice(1),
            next.text.slice(1),
          );
          if (changed) {
            if (changed.removedTo > changed.removedFrom) {
              decorations.push(
                Decoration.mark({ class: "cowboy-diff-word-removed" }).range(
                  line.from + 1 + changed.removedFrom,
                  line.from + 1 + changed.removedTo,
                ),
              );
            }
            if (changed.addedTo > changed.addedFrom) {
              decorations.push(
                Decoration.mark({ class: "cowboy-diff-word-added" }).range(
                  next.from + 1 + changed.addedFrom,
                  next.from + 1 + changed.addedTo,
                ),
              );
            }
          }
        }
      }
      if (line.to >= view.state.doc.length) break;
      position = line.to + 1;
    }
  }
  return Decoration.set(decorations, true);
}

const diffView = ViewPlugin.fromClass(
  class {
    decorations;

    constructor(view: EditorView) {
      this.decorations = diffDecorations(view);
    }

    update(update: ViewUpdate): void {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = diffDecorations(update.view);
      }
    }
  },
  { decorations: (value) => value.decorations },
);

const expandContext = StateEffect.define<number>();

class ContextFoldWidget extends WidgetType {
  constructor(
    private readonly hiddenLines: number,
    private readonly from: number,
  ) {
    super();
  }

  toDOM(view: EditorView): HTMLElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "cowboy-diff-context-fold";
    button.textContent = `⋯ ${this.hiddenLines} unchanged lines`;
    button.addEventListener("click", () => {
      view.dispatch({ effects: expandContext.of(this.from) });
    });
    return button;
  }
}

interface ContextFoldState {
  decorations: DecorationSet;
  expanded: ReadonlySet<number>;
}

function buildContextDecorations(
  state: EditorState,
  expanded: ReadonlySet<number>,
): DecorationSet {
  const ranges = diffContextFolds(state.doc.toString())
    .map((fold) => {
      const from = state.doc.line(fold.fromLine).from;
      const to = state.doc.line(fold.toLine).to;
      if (expanded.has(from)) return undefined;
      return Decoration.replace({
        block: true,
        widget: new ContextFoldWidget(fold.hiddenLines, from),
      }).range(from, to);
    })
    .filter((range) => range !== undefined);
  return Decoration.set(ranges, true);
}

const contextFolding = StateField.define<ContextFoldState>({
  create(state) {
    const expanded = new Set<number>();
    return {
      decorations: buildContextDecorations(state, expanded),
      expanded,
    };
  },
  update(value, transaction) {
    const expanded = transaction.docChanged
      ? new Set<number>()
      : new Set(value.expanded);
    let changed = transaction.docChanged;
    for (const effect of transaction.effects) {
      if (effect.is(expandContext)) {
        expanded.add(effect.value);
        changed = true;
      }
    }
    return changed
      ? {
        decorations: buildContextDecorations(transaction.state, expanded),
        expanded,
      }
      : value;
  },
  provide: (field) =>
    EditorView.decorations.from(field, (value) => value.decorations),
});

export default function CodeViewer({
  text,
  kind,
  path,
  softWrap,
  fontSize,
  revealLine,
}: {
  text: string;
  kind: "source" | "diff";
  path: string;
  softWrap: boolean;
  fontSize: number;
  revealLine?: number | undefined;
}): React.JSX.Element {
  const theme = useTheme();
  const editorRef = useRef<EditorView | null>(null);
  const [language, setLanguage] = useState<LanguageSupport | null>(null);

  useEffect(() => {
    let current = true;
    setLanguage(null);
    if (kind === "source") {
      void loadCodeLanguage(path).then((support) => {
        if (current) setLanguage(support);
      });
    }
    return () => {
      current = false;
    };
  }, [kind, path]);

  const extensions = useMemo(() => {
    const values: Extension[] = [
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      cmTheme(theme, true),
      EditorView.theme({
        "&": { height: "100%", fontSize: `${fontSize}px` },
        ".cm-scroller": {
          fontSize: `${fontSize}px`,
          overflow: "auto",
          overflowX: softWrap ? "hidden" : "auto",
          WebkitOverflowScrolling: "touch",
        },
        ".cm-content": {
          fontSize: `${fontSize}px`,
          padding: "12px 0 48px",
          minWidth: softWrap ? 0 : "max-content",
          width: softWrap ? "100%" : "max-content",
        },
        ".cm-line": { padding: "0 12px" },
        ".cm-gutters": {
          backgroundColor: theme.palette.background.default,
          borderRightColor: theme.palette.divider,
        },
        ".cowboy-diff-added": {
          backgroundColor: theme.palette.mode === "dark"
            ? "rgba(46, 160, 67, 0.18)"
            : "rgba(46, 160, 67, 0.12)",
        },
        ".cowboy-diff-removed": {
          backgroundColor: theme.palette.mode === "dark"
            ? "rgba(248, 81, 73, 0.18)"
            : "rgba(248, 81, 73, 0.12)",
        },
        ".cowboy-diff-hunk": {
          color: theme.palette.primary.main,
          backgroundColor: theme.palette.action.hover,
        },
        ".cowboy-diff-word-added": {
          backgroundColor: theme.palette.mode === "dark"
            ? "rgba(46, 160, 67, 0.42)"
            : "rgba(46, 160, 67, 0.28)",
          borderRadius: "2px",
        },
        ".cowboy-diff-word-removed": {
          backgroundColor: theme.palette.mode === "dark"
            ? "rgba(248, 81, 73, 0.42)"
            : "rgba(248, 81, 73, 0.26)",
          borderRadius: "2px",
        },
        ".cowboy-diff-context-fold": {
          width: "100%",
          minHeight: "36px",
          border: "0",
          borderTop: `1px solid ${theme.palette.divider}`,
          borderBottom: `1px solid ${theme.palette.divider}`,
          color: theme.palette.text.secondary,
          background: theme.palette.action.hover,
          font: "inherit",
          textAlign: "left",
          padding: "0 12px",
        },
      }),
    ];
    if (softWrap) values.push(EditorView.lineWrapping);
    if (language) values.push(language);
    if (kind === "diff") values.push(diffView, contextFolding);
    return values;
  }, [fontSize, kind, language, softWrap, theme]);

  useEffect(() => {
    const view = editorRef.current;
    if (!view || revealLine === undefined) return;
    const line = view.state.doc.line(
      Math.max(1, Math.min(revealLine, view.state.doc.lines)),
    );
    view.dispatch({
      effects: EditorView.scrollIntoView(line.from, {
        y: "start",
        yMargin: 12,
      }),
    });
  }, [revealLine, text]);

  return (
    <Box
      sx={{
        height: "100%",
        minHeight: 0,
        "& > div": { height: "100%" },
        // `cmTheme` is shared with the Agent composer and intentionally follows
        // the global reading scale. Code Review owns a separate code-only
        // setting, so enforce it at this product boundary instead of changing
        // the shared theme. `!important` is required because CodeMirror mounts
        // theme style modules in an order that can change after a lazy grammar
        // reconfiguration.
        "& .cm-editor, & .cm-scroller, & .cm-content": {
          fontSize: `${fontSize}px !important`,
        },
      }}
    >
      <CodeMirror
        value={text}
        extensions={extensions}
        onCreateEditor={(view) => {
          editorRef.current = view;
        }}
        basicSetup={{
          lineNumbers: true,
          foldGutter: false,
          highlightActiveLine: false,
          highlightActiveLineGutter: false,
          autocompletion: false,
          bracketMatching: false,
          closeBrackets: false,
          searchKeymap: false,
        }}
        editable={false}
        theme="none"
        height="100%"
      />
    </Box>
  );
}
