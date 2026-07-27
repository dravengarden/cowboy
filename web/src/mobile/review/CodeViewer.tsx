import {
  EditorState,
  type Extension,
  type Range,
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
import {
  HighlightStyle,
  type LanguageSupport,
  syntaxHighlighting,
} from "@codemirror/language";
import { highlightTree, tags } from "@lezer/highlight";
import { cmTheme } from "../../cmTheme";
import { loadCodeLanguage } from "./codeLanguage";
import { codeSyntaxPalette } from "./codeSyntaxTheme";
import { changedWordRange } from "./diffWordModel";
import { diffContextFolds } from "./diffContextModel";
import {
  diffPointToNewFile,
  diffSourceProjection,
} from "./diffSourceModel";
import type { CodeLanguage } from "./codeApi";

export interface CodeInspectCandidate {
  label: string;
  row: number;
  column: number;
}

class InlayHintWidget extends WidgetType {
  constructor(private readonly label: string) {
    super();
  }

  toDOM(): HTMLElement {
    const span = document.createElement("span");
    span.className = "cowboy-inlay-hint";
    span.textContent = this.label;
    return span;
  }
}

function utf8OffsetToDocumentOffset(text: string, byteOffset: number): number {
  if (byteOffset <= 0) return 0;
  const bytes = new TextEncoder().encode(text);
  const end = Math.min(byteOffset, bytes.length);
  return new TextDecoder().decode(bytes.slice(0, end)).length;
}

function languageDecorations(
  state: EditorState,
  language: CodeLanguage | undefined,
  diagnostics: boolean,
  inlayHints: boolean,
  semanticHighlighting: boolean,
): DecorationSet {
  if (!language) return Decoration.none;
  const ranges: Range<Decoration>[] = [];
  if (semanticHighlighting) {
    let row = 0;
    let column = 0;
    for (let index = 0; index + 4 < language.semanticTokens.length; index += 5) {
      const rowDelta = language.semanticTokens[index] ?? 0;
      const columnDelta = language.semanticTokens[index + 1] ?? 0;
      row += rowDelta;
      column = rowDelta === 0 ? column + columnDelta : columnDelta;
      if (row >= state.doc.lines) continue;
      const line = state.doc.line(row + 1);
      const from = Math.min(line.to, line.from + column);
      const to = Math.min(
        line.to,
        from + (language.semanticTokens[index + 2] ?? 0),
      );
      if (to > from) {
        ranges.push(
          Decoration.mark({
            class: `cowboy-semantic-token cowboy-semantic-${
              (language.semanticTokens[index + 3] ?? 0) % 6
            }`,
          }).range(from, to),
        );
      }
    }
  }
  if (diagnostics) {
    for (const diagnostic of language.diagnostics) {
      if (diagnostic.start.row >= state.doc.lines) continue;
      const startLine = state.doc.line(diagnostic.start.row + 1);
      const endLine = state.doc.line(
        Math.min(state.doc.lines, diagnostic.end.row + 1),
      );
      const from = Math.min(startLine.to, startLine.from + diagnostic.start.column);
      const to = Math.max(
        from,
        Math.min(endLine.to, endLine.from + diagnostic.end.column),
      );
      const diagnosticTo = Math.min(
        state.doc.length,
        Math.max(from < state.doc.length ? from + 1 : from, to),
      );
      if (diagnosticTo <= from) continue;
      ranges.push(
        Decoration.mark({
          class: `cowboy-diagnostic cowboy-diagnostic-${diagnostic.severity}`,
          attributes: { title: diagnostic.message },
        }).range(from, diagnosticTo),
      );
    }
  }
  if (inlayHints) {
    for (const hint of language.inlayHints) {
      const position = Math.min(
        state.doc.length,
        utf8OffsetToDocumentOffset(state.doc.toString(), hint.offset),
      );
      ranges.push(
        Decoration.widget({
          widget: new InlayHintWidget(hint.label),
          side: 1,
        }).range(position),
      );
    }
  }
  return Decoration.set(ranges, true);
}

function diffLanguageDecorations(
  text: string,
  language: LanguageSupport,
  highlighter: HighlightStyle,
): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  const projection = diffSourceProjection(text);
  const tree = language.language.parser.parse(projection);
  highlightTree(tree, highlighter, (from, to, classes) => {
    if (to > from) {
      ranges.push(Decoration.mark({ class: classes }).range(from, to));
    }
  });
  return Decoration.set(ranges, true);
}

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
  languageData,
  diagnostics,
  inlayHints,
  semanticHighlighting,
  onInspect,
}: {
  text: string;
  kind: "source" | "diff";
  path: string;
  softWrap: boolean;
  fontSize: number;
  revealLine?: number | undefined;
  languageData?: CodeLanguage | undefined;
  diagnostics: boolean;
  inlayHints: boolean;
  semanticHighlighting: boolean;
  onInspect?: ((
    candidates: CodeInspectCandidate[],
    anchor: { top: number; left: number },
  ) => void) | undefined;
}): React.JSX.Element {
  const theme = useTheme();
  const editorRef = useRef<EditorView | null>(null);
  const [language, setLanguage] = useState<LanguageSupport | null>(null);

  useEffect(() => {
    let current = true;
    setLanguage(null);
    void loadCodeLanguage(path).then((support) => {
      if (current) setLanguage(support);
    });
    return () => {
      current = false;
    };
  }, [kind, path]);

  const extensions = useMemo(() => {
    const syntax = codeSyntaxPalette(theme.palette.mode);
    const highlightStyle = HighlightStyle.define([
      { tag: tags.keyword, color: syntax.keyword, fontWeight: "600" },
      {
        tag: [tags.string, tags.special(tags.string), tags.regexp, tags.escape],
        color: syntax.string,
      },
      {
        tag: [tags.number, tags.bool, tags.null],
        color: syntax.number,
      },
      {
        tag: [tags.typeName, tags.className, tags.namespace],
        color: syntax.type,
      },
      {
        tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
        color: syntax.function,
      },
      {
        tag: [tags.propertyName, tags.attributeName, tags.labelName],
        color: syntax.property,
      },
      {
        tag: [tags.constant(tags.name), tags.standard(tags.name), tags.local(tags.name)],
        color: syntax.constant,
      },
      {
        tag: [tags.operator, tags.punctuation, tags.bracket],
        color: syntax.operator,
      },
      {
        tag: [tags.comment, tags.docComment, tags.meta],
        color: syntax.muted,
        fontStyle: "italic",
      },
      { tag: [tags.tagName, tags.heading], color: syntax.tag },
      {
        tag: [tags.variableName, tags.name],
        color: syntax.foreground,
      },
      {
        tag: tags.invalid,
        color: syntax.invalid,
        textDecoration: "underline wavy",
      },
    ]);
    const values: Extension[] = [
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      cmTheme(theme, true),
      syntaxHighlighting(highlightStyle),
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
        ".cowboy-inlay-hint": {
          color: theme.palette.text.secondary,
          background: theme.palette.action.hover,
          borderRadius: "4px",
          fontSize: "0.82em",
          marginLeft: "4px",
          padding: "1px 4px",
        },
        ".cowboy-diagnostic": {
          textDecorationLine: "underline",
          textDecorationStyle: "wavy",
          textUnderlineOffset: "3px",
        },
        ".cowboy-diagnostic-1": { textDecorationColor: theme.palette.error.main },
        ".cowboy-diagnostic-2": {
          textDecorationColor: theme.palette.warning.main,
        },
        ".cowboy-diagnostic-3, .cowboy-diagnostic-4": {
          textDecorationColor: theme.palette.info.main,
        },
        ".cowboy-semantic-0": { color: syntax.keyword },
        ".cowboy-semantic-1": { color: syntax.type },
        ".cowboy-semantic-2": { color: syntax.function },
        ".cowboy-semantic-3": { color: syntax.string },
        ".cowboy-semantic-4": { color: syntax.number },
        ".cowboy-semantic-5": {
          color: syntax.property,
          fontStyle: "italic",
        },
      }),
    ];
    if (softWrap) values.push(EditorView.lineWrapping);
    if (kind === "source" && language) values.push(language);
    if (kind === "diff") values.push(diffView, contextFolding);
    if (kind === "diff" && language) {
      values.push(
        EditorView.decorations.of(
          diffLanguageDecorations(text, language, highlightStyle),
        ),
      );
    }
    if (kind === "source" && languageData) {
      values.push(
        EditorView.decorations.of(
          languageDecorations(
            EditorState.create({ doc: text }),
            languageData,
            diagnostics,
            inlayHints,
            semanticHighlighting,
          ),
        ),
      );
    }
    if (onInspect) {
      values.push(
        EditorView.domEventHandlers({
          click: (event, view) => {
            const selection = globalThis.getSelection?.();
            if (selection && !selection.isCollapsed) return false;
            const offset = view.posAtCoords({
              x: event.clientX,
              y: event.clientY,
            });
            if (offset === null) return false;
            const line = view.state.doc.lineAt(offset);
            const column = offset - line.from;
            const candidates = Array.from(
              line.text.matchAll(/[\p{L}\p{N}_$]+/gu),
            ).flatMap((match) => {
              const start = match.index;
              const end = start + match[0].length;
              const from = line.from + start;
              const to = line.from + end;
              const fromCoords = view.coordsAtPos(from);
              const toCoords = view.coordsAtPos(to);
              if (!fromCoords || !toCoords) return [];
              const left = Math.min(fromCoords.left, toCoords.left);
              const right = Math.max(fromCoords.right, toCoords.right);
              const distance = event.clientX < left
                ? left - event.clientX
                : event.clientX > right
                ? event.clientX - right
                : 0;
              if (distance > 36) return [];
              const sourcePoint = kind === "diff"
                ? diffPointToNewFile(text, line.number - 1, start)
                : { row: line.number - 1, column: start };
              return sourcePoint
                ? [{
                  candidate: {
                    label: match[0],
                    ...sourcePoint,
                  },
                  distance,
                  contains: column >= start && column <= end,
                }]
                : [];
            }).sort((a, b) =>
              Number(b.contains) - Number(a.contains) ||
              a.distance - b.distance ||
              a.candidate.column - b.candidate.column
            ).slice(0, 5).map(({ candidate }) => candidate);
            if (candidates.length === 0) return false;
            event.preventDefault();
            onInspect(candidates, {
              top: event.clientY,
              left: event.clientX,
            });
            return true;
          },
        }),
      );
    }
    return values;
  }, [
    diagnostics,
    fontSize,
    inlayHints,
    kind,
    language,
    languageData,
    onInspect,
    semanticHighlighting,
    softWrap,
    text,
    theme,
  ]);

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
