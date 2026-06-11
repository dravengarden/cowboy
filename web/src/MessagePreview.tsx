import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { Box, Button, useTheme } from "@mui/material";
import CodeMirror from "@uiw/react-codemirror";
import type { Extension } from "@codemirror/state";
import { cmTheme } from "./cmTheme";
import { livePreviewExtensions } from "./composerExtensions";

// Read-only live-preview of a queued/draft message, rendered with the EXACT same
// mdlive engine as the input composer (`livePreviewExtensions` + `cmTheme`) — so
// the markdown a queued/draft row shows is byte-for-byte the look you composed
// (bold, lists, tasks, highlight, …), not raw `**`/`- ` text. The editor is
// non-interactive (`editable={false}` + `pointer-events: none`), so a tap falls
// through to `onClick` → open the row's edit; a long note clamps to a few lines
// with a Show more / less toggle (the "默认折叠" ask), measured like the old
// ClampedText. Image tokens are stripped by the caller (attachments render as
// chips below), so no inline-image registry is needed here.
const COLLAPSED_MAX = "4.6em"; // ~3 lines at the composer's 1.5 line-height

export function MessagePreview({
  text,
  onClick,
}: {
  text: string;
  onClick?: () => void;
}): React.JSX.Element {
  const theme = useTheme();
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // The SAME extensions the input uses (reused, not re-declared, so the preview
  // can never visually drift from the editor). Memoised per theme.
  const extensions = useMemo<Extension[]>(
    () => [cmTheme(theme), ...livePreviewExtensions()],
    [theme],
  );

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || expanded) return undefined;
    const measure = (): void => setOverflowing(el.scrollHeight > el.clientHeight + 1);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [text, expanded]);

  return (
    <>
      <Box
        ref={ref}
        onClick={onClick}
        sx={{
          ...(onClick && { cursor: "pointer" }),
          ...(expanded ? {} : { maxHeight: COLLAPSED_MAX, overflow: "hidden" }),
          // The preview is non-interactive — taps select the row to edit, they
          // don't place a caret or follow a link inside the read-only editor.
          "& .cm-editor": { backgroundColor: "transparent", pointerEvents: "none" },
          "& .cm-content": { padding: 0 },
          "& .cm-scroller": { lineHeight: "var(--cowboy-reading-line-height, 1.5)" },
        }}
      >
        <CodeMirror
          value={text}
          editable={false}
          theme="none"
          basicSetup={false}
          extensions={extensions}
        />
      </Box>
      {(overflowing || expanded) && (
        <Button
          size="small"
          onClick={(e): void => {
            e.stopPropagation();
            setExpanded((v) => !v);
          }}
          sx={{
            textTransform: "none",
            minWidth: 0,
            px: 0.5,
            py: 0,
            mt: 0.25,
            color: "text.secondary",
            fontWeight: 400,
          }}
        >
          {expanded ? "Show less" : "Show more"}
        </Button>
      )}
    </>
  );
}
