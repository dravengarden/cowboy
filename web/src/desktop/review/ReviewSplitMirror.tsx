import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { syntaxHighlighting, type LanguageSupport } from "@codemirror/language";
import { Box, useTheme } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { cmTheme } from "../../cmTheme";
import { loadCodeLanguage } from "../../mobile/review/codeLanguage";
import { reviewHighlightStyle } from "../../mobile/review/codeSyntaxTheme";
import type { SplitSide } from "./splitDiffModel";
import { splitSideExtensions } from "./splitSideExtensions";

/**
 * The old side of a split diff: a passive, non-focusable reader that follows
 * the new side's scroll row for row. Keys and selection belong to the new
 * side, which carries the Vim buffer.
 */
export function ReviewSplitMirror({
  path,
  side,
  fontSize,
  scrollTop,
}: {
  path: string;
  side: SplitSide;
  fontSize: number;
  scrollTop: number;
}): React.JSX.Element {
  const theme = useTheme();
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const [support, setSupport] = useState<LanguageSupport | null>(null);
  useEffect(() => {
    let current = true;
    void loadCodeLanguage(path, side.text.slice(0, 200)).then((language) => {
      if (current) setSupport(language);
    });
    return () => {
      current = false;
    };
  }, [path, side.text]);
  useEffect(() => {
    if (!host.current) return undefined;
    const editor = new EditorView({ parent: host.current });
    view.current = editor;
    return () => {
      view.current = null;
      editor.destroy();
    };
  }, []);
  useEffect(() => {
    view.current?.setState(EditorState.create({
      doc: side.text,
      extensions: [
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        cmTheme(theme, true),
        syntaxHighlighting(reviewHighlightStyle(theme.palette.mode)),
        splitSideExtensions(theme, side),
        ...(support ? [support] : []),
        EditorView.theme({
          "&": { height: "100%", fontSize: `${String(fontSize)}px` },
          ".cm-scroller": { overflow: "hidden", fontSize: `${String(fontSize)}px` },
          ".cm-content": { padding: "8px 0 40vh" },
          ".cm-line": { padding: "0 12px" },
          ".cm-gutters": { backgroundColor: theme.palette.background.default, borderRightColor: theme.palette.divider },
        }),
      ],
    }));
  }, [fontSize, side, support, theme]);
  useEffect(() => {
    if (view.current) view.current.scrollDOM.scrollTop = scrollTop;
  }, [scrollTop]);
  return (
    <Box
      ref={host}
      aria-hidden
      data-desktop-review-split-old
      sx={{ flex: 1, minWidth: 0, minHeight: 0, borderRight: 1, borderColor: "divider", "& .cm-editor": { height: "100%" } }}
    />
  );
}
