import { EditorState, type Extension } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import CodeMirror from "@uiw/react-codemirror";
import { Box, useTheme } from "@mui/material";
import { useEffect, useMemo, useRef } from "react";
import { cmTheme } from "../../cmTheme";

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

export default function CodeViewer({
  text,
  kind,
  softWrap,
  revealLine,
}: {
  text: string;
  kind: "source" | "diff";
  softWrap: boolean;
  revealLine?: number | undefined;
}): React.JSX.Element {
  const theme = useTheme();
  const editorRef = useRef<EditorView | null>(null);
  const extensions = useMemo(() => {
    const values: Extension[] = [
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      cmTheme(theme, true),
      EditorView.theme({
        "&": { height: "100%", fontSize: "0.875rem" },
        ".cm-scroller": {
          overflow: "auto",
          WebkitOverflowScrolling: "touch",
        },
        ".cm-content": {
          padding: "12px 0 48px",
          minWidth: "max-content",
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
      }),
    ];
    if (softWrap) values.push(EditorView.lineWrapping);
    if (kind === "diff") values.push(diffView);
    return values;
  }, [kind, softWrap, theme]);

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
    <Box sx={{ height: "100%", minHeight: 0, "& > div": { height: "100%" } }}>
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
