import { memo, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Box, Button, useTheme } from "@mui/material";
import { ExpandLess, ExpandMore } from "@mui/icons-material";
import CodeMirror from "@uiw/react-codemirror";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { cmTheme } from "./cmTheme";
import { livePreviewExtensions } from "./composerExtensions";
import { useReliableTouchTap } from "./useReliableTouchTap";

// Read-only live-preview of a queued/draft message, rendered with the EXACT same
// mdlive engine as the input composer (`livePreviewExtensions` + `cmTheme`) — so
// the markdown a queued/draft row shows is byte-for-byte the look you composed
// (bold, lists, tasks, highlight, …), not raw `**`/`- ` text. The editor is
// non-interactive (`editable={false}` + `pointer-events: none`), so a tap falls
// through to `onClick` → open the row's edit; a long note clamps to a few lines
// with a Show more / less toggle (the "默认折叠" ask), measured like the old
// ClampedText. Image tokens are stripped by the caller (attachments render as
// chips below), so no inline-image registry is needed here.
const COLLAPSED_MAX = "3.1em"; // ~2 lines at the composer's 1.5 line-height

// Display-only whitespace tidy for the preview — like HTML's whitespace
// collapsing, but markdown-aware (the STORED text is untouched; this only shapes
// what the read-only preview shows). Trim leading/trailing blank space, cap a run
// of blank lines at one, and collapse interior space runs WITHIN each line —
// while PRESERVING each line's leading indentation so nested lists / indented
// code still render. Without this a draft with leading newlines wastes the
// preview's first lines on emptiness ("预览前后留白").
function compactForPreview(text: string): string {
  return text
    .split("\n")
    .map((line) => {
      const indent = line.match(/^[ \t]*/)?.[0] ?? "";
      return indent + line.slice(indent.length).replace(/[ \t]+/g, " ").trimEnd();
    })
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function MessagePreviewImpl({
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
  // iOS may dispatch the compatibility click only after WebKit has closed the
  // trusted user-activation window.  Queue/Draft edit needs to mount and focus
  // its textarea inside that window or the card changes state without raising
  // the keyboard.  Resolve an actual tap on pointer-up (while rejecting scroll
  // gestures and nested controls), and suppress the duplicate click.
  const editTap = useReliableTouchTap<HTMLDivElement>(() => onClick?.());

  // The SAME extensions the input uses (reused, not re-declared, so the preview
  // can never visually drift from the editor). Memoised per theme.
  const extensions = useMemo<Extension[]>(
    () => [
      cmTheme(theme),
      ...livePreviewExtensions(),
      EditorView.contentAttributes.of({ tabindex: "-1" }),
    ],
    [theme],
  );
  const display = useMemo(() => compactForPreview(text), [text]);

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
    <Box sx={{ position: "relative", minWidth: 0 }}>
      <Box
        ref={ref}
        {...(onClick ? editTap : {})}
        sx={{
          ...(onClick && { cursor: "pointer" }),
          ...(expanded
            ? {}
            : {
              maxHeight: COLLAPSED_MAX,
              overflow: "hidden",
              // A hard crop made the last line look broken. Fade the final line
              // so the compact disclosure below reads as its continuation rather
              // than as the former loose 40px MUI button row.
              maskImage: "linear-gradient(to bottom, #000 0, #000 calc(100% - 1.35em), transparent 100%)",
              WebkitMaskImage:
                "linear-gradient(to bottom, #000 0, #000 calc(100% - 1.35em), transparent 100%)",
            }),
          // The preview is non-interactive — taps select the row to edit, they
          // don't place a caret or follow a link inside the read-only editor.
          "& .cm-editor": { backgroundColor: "transparent", pointerEvents: "none" },
          "& .cm-content": {
            padding: 0,
            caretColor: "transparent",
            userSelect: "none",
            WebkitUserSelect: "none",
          },
          "& .cm-scroller": { lineHeight: "var(--cowboy-reading-line-height, 1.5)" },
        }}
      >
        <CodeMirror
          value={display}
          editable={false}
          theme="none"
          basicSetup={false}
          extensions={extensions}
        />
      </Box>
      {(overflowing || expanded) && (
        <Button
          size="small"
          endIcon={expanded
            ? <ExpandLess sx={{ fontSize: 16 }} />
            : <ExpandMore sx={{ fontSize: 16 }} />}
          onClick={(e): void => {
            e.stopPropagation();
            setExpanded((v) => !v);
          }}
          sx={{
            textTransform: "none",
            minWidth: 0,
            // Touch theme globally raises every Button to 40px. This disclosure
            // supplies its own 44px pseudo hit target below, so keep the painted
            // label compact instead of paying for both policies.
            minHeight: "28px !important",
            height: 28,
            px: 0.5,
            py: 0,
            mt: expanded ? 0.25 : 0,
            color: "text.secondary",
            fontWeight: 400,
            lineHeight: 1.2,
            borderRadius: 0.75,
            "& .MuiButton-endIcon": { ml: 0.125, mr: -0.375 },
            // Keep a genuine 44px touch target without making the painted
            // disclosure row 44px tall. It remains in normal flow, so headings
            // and lists can never be obscured by the control.
            "&::before": {
              content: '""',
              position: "absolute",
              inset: -8,
            },
          }}
        >
          {expanded ? "Show less" : "Show more"}
        </Button>
      )}
    </Box>
  );
}

// Each preview mounts a FULL CodeMirror (mdlive fidelity — the same engine as the
// composer), so a queue/drafts panel can hold a dozen+ heavy editors at once.
// Memoize on `text` — the only visual input — so a parent re-render (a store tick
// mid-stream, a collapse toggle, a reorder) does NOT reconcile every CM6 instance,
// which was the source of the expand/collapse jank. `onClick` is a stable per-row
// closure (opens that row's edit); its identity churning each render must not
// re-render the editor, so the comparison checks only its presence, not identity.
export const MessagePreview = memo(
  MessagePreviewImpl,
  (a, b) => a.text === b.text && !!a.onClick === !!b.onClick,
);
