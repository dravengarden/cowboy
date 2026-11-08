import { type Extension, EditorState } from "@codemirror/state";
import { EditorView, ViewPlugin } from "@codemirror/view";
import { syntaxHighlighting, type LanguageSupport } from "@codemirror/language";
import { getCM, Vim } from "@replit/codemirror-vim";
import { Box, useTheme } from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { cmTheme } from "../../cmTheme";
import { notify } from "../../store";
import { getVimConfigSource } from "../../vimSetting";
import { loadCodeLanguage } from "../../mobile/review/codeLanguage";
import { reviewHighlightStyle } from "../../mobile/review/codeSyntaxTheme";
import { reviewLineGutter } from "../../mobile/review/reviewLineGutter";
import { createImeAutoInsertVim } from "../vim/imeAutoInsertVim";
import { parseVimConfig } from "../vim/vimConfig";
import { vimCommandKey } from "../vim/vimCommandKey";
import { reviewDiffDecorations } from "./reviewDiffDecorations";
import { revealEditorLine } from "./reviewEditorSelection";
import {
  type ReviewDocumentAction,
  reviewDocumentAction,
  userMapsSequence,
} from "./reviewDocumentKeys";

interface VimInputState {
  keyBuffer: string[];
  prefixRepeat: string[];
  motionRepeat: string[];
  operator?: unknown;
}

/** Review's keys, resolved per editor before the Vim command sink sees them. */
function reviewKeys(onAction: (action: ReviewDocumentAction, view: EditorView) => void): Extension {
  return ViewPlugin.define((view) => {
    const listener = (event: KeyboardEvent): void => {
      const cm = getCM(view);
      const vim = cm?.state?.vim as
        | { insertMode?: boolean; visualMode?: boolean; inputState?: VimInputState }
        | undefined;
      if (!cm || !vim?.inputState || vim.insertMode || view.composing) return;
      const key = vimCommandKey(event);
      if (!key) return;
      const input = vim.inputState;
      const mappings = parseVimConfig(getVimConfigSource()).mappings;
      const action = reviewDocumentAction(
        {
          keyBuffer: input.keyBuffer.join(""),
          operator: Boolean(input.operator),
          count: Number.parseInt(input.prefixRepeat.join(""), 10) || 0,
          visual: Boolean(vim.visualMode),
        },
        key,
        (sequence, visual) => userMapsSequence(mappings, sequence, visual),
      );
      if (!action) return;
      event.preventDefault();
      event.stopPropagation();
      input.keyBuffer.length = 0;
      input.prefixRepeat = [];
      input.motionRepeat = [];
      onAction(action, view);
      if (vim.visualMode) Vim.handleKey(cm, "<Esc>", "user");
    };
    view.dom.addEventListener("keydown", listener, true);
    // A read-only buffer has no Insert mode: leave it at once, and say why.
    let connected: ReturnType<typeof getCM> = null;
    const onMode = (event: { mode?: string }): void => {
      if (event.mode !== "insert") return;
      queueMicrotask(() => {
        if (connected) Vim.handleKey(connected, "<Esc>", "user");
      });
      notify("Review is read-only: send the lines to the Prompt with Enter", "info");
    };
    const connect = (): void => {
      connected = getCM(view);
      connected?.on?.("vim-mode-change", onMode);
    };
    queueMicrotask(connect);
    return {
      destroy: () => {
        view.dom.removeEventListener("keydown", listener, true);
        connected?.off?.("vim-mode-change", onMode);
      },
    };
  });
}

/**
 * The Document: CodeMirror 6 with Cowboy's Desktop Vim runtime (IME-safe
 * Normal-mode command sink, the user's vimrc), read-only. A unified diff gets
 * structural line backgrounds and new-side line numbers; a source file its
 * grammar, loaded on demand.
 */
export function ReviewCodeEditor({
  documentKey,
  path,
  kind,
  text,
  wrap,
  fontSize,
  restoreLine,
  onAction,
  onCursorLine,
  onView,
}: {
  documentKey: string;
  path: string;
  kind: "source" | "diff";
  text: string;
  wrap: boolean;
  fontSize: number;
  restoreLine: number | undefined;
  onAction: (action: ReviewDocumentAction, view: EditorView) => void;
  onCursorLine: (line: number) => void;
  onView: (view: EditorView | null) => void;
}): React.JSX.Element {
  const theme = useTheme();
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const actions = useRef({ onAction, onCursorLine });
  actions.current = { onAction, onCursorLine };
  const [language, setLanguage] = useState<{ key: string; support: LanguageSupport | null }>();
  useEffect(() => {
    if (kind !== "source") return undefined;
    let current = true;
    void loadCodeLanguage(path, text.slice(0, 200)).then((support) => {
      if (current) setLanguage({ key: documentKey, support });
    });
    return () => {
      current = false;
    };
  }, [documentKey, kind, path, text]);
  const support = language?.key === documentKey ? language.support : null;

  const extensions = useMemo((): Extension[] => [
    EditorState.readOnly.of(true),
    createImeAutoInsertVim().extension,
    reviewKeys((action, editor) => actions.current.onAction(action, editor)),
    cmTheme(theme, true),
    syntaxHighlighting(reviewHighlightStyle(theme.palette.mode)),
    reviewLineGutter(kind, (row) => {
      const editor = view.current;
      if (editor) revealEditorLine(editor, row);
      return true;
    }),
    ...(kind === "diff" ? [reviewDiffDecorations(theme)] : support ? [support] : []),
    ...(wrap ? [EditorView.lineWrapping] : []),
    EditorView.updateListener.of((update) => {
      if (update.selectionSet || update.docChanged) {
        actions.current.onCursorLine(update.state.doc.lineAt(update.state.selection.main.head).number);
      }
    }),
    EditorView.theme({
      "&": { height: "100%", fontSize: `${String(fontSize)}px` },
      ".cm-scroller": { overflow: "auto", fontSize: `${String(fontSize)}px` },
      ".cm-content": { padding: "8px 0 40vh" },
      ".cm-line": { padding: "0 12px" },
      ".cm-gutters": {
        backgroundColor: theme.palette.background.default,
        borderRightColor: theme.palette.divider,
      },
    }),
  ], [fontSize, kind, support, theme, wrap]);

  useEffect(() => {
    if (!host.current) return undefined;
    const editor = new EditorView({ parent: host.current });
    view.current = editor;
    onView(editor);
    return () => {
      onView(null);
      view.current = null;
      editor.destroy();
    };
    // The view lives as long as the host; documents replace its state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const hadFocus = editor.dom.contains(document.activeElement);
    editor.setState(EditorState.create({ doc: text, extensions }));
    if (restoreLine !== undefined) revealEditorLine(editor, restoreLine);
    if (hadFocus) editor.focus();
    // `restoreLine` belongs to the document that just arrived.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [documentKey, extensions, text]);

  return (
    <Box
      ref={host}
      data-desktop-review-editor={kind}
      sx={{ flex: 1, minHeight: 0, minWidth: 0, "& .cm-editor": { height: "100%" } }}
    />
  );
}
