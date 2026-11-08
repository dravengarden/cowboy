import type { EditorView } from "@codemirror/view";
import { Box, ListItemText, Menu, MenuItem, Popover, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { notify } from "../../store";
import type { CodeHover, CodeLocation } from "../../mobile/review/codeApi";
import { revealEditorLine } from "./reviewEditorSelection";
import type { ReviewDocumentAction } from "./reviewDocumentKeys";
import type { ReviewLanguage } from "./useReviewLanguage";
import type { ReviewWorkbench } from "./useReviewWorkbench";

type Anchor = { top: number; left: number };

const NAVIGATION_LABEL = {
  definition: "definition",
  declaration: "declaration",
  typeDefinition: "type definition",
  implementation: "implementation",
  references: "references",
} as const;

function cursorPoint(view: EditorView): { row: number; column: number; anchor: Anchor } {
  const head = view.state.selection.main.head;
  const line = view.state.doc.lineAt(head);
  const coords = view.coordsAtPos(head);
  return {
    row: line.number - 1,
    column: head - line.from,
    anchor: { top: coords?.bottom ?? 0, left: coords?.left ?? 0 },
  };
}

function HoverBody({ hover }: { hover: CodeHover | "loading" | "error" }): React.JSX.Element {
  if (hover === "loading") return <Typography sx={{ fontSize: "0.78rem", color: "text.secondary" }}>Asking the language server…</Typography>;
  if (hover === "error" || hover.contents.length === 0) {
    return <Typography sx={{ fontSize: "0.78rem", color: "text.secondary" }}>Nothing is known about this symbol.</Typography>;
  }
  return (
    <>
      {hover.contents.map((block, index) => (
        <Box
          key={index}
          component={block.language ? "pre" : "div"}
          sx={{
            m: 0,
            mb: 1,
            fontSize: "0.78rem",
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
            ...(block.language ? { fontFamily: "monospace", bgcolor: "action.hover", p: 1, borderRadius: 1 } : {}),
          }}
        >
          {block.text}
        </Box>
      ))}
    </>
  );
}

/**
 * `K` and the `g` navigation keys over the open source file. Hover opens a
 * popover at the cursor (`Esc` closes it, back to the Document); one location
 * opens there, several open a menu (Vim menu keys). A diff has no file
 * coordinates here, so it points to the file instead.
 */
export function ReviewLanguageLayer({ review, language }: {
  review: ReviewWorkbench;
  language: ReviewLanguage;
}): React.JSX.Element {
  const [hover, setHover] = useState<{ anchor: Anchor; value: CodeHover | "loading" | "error" }>();
  const [locations, setLocations] = useState<{ anchor: Anchor; items: readonly CodeLocation[]; label: string }>();
  const pending = useRef<AbortController | undefined>(undefined);
  const latest = useRef({ review, language });
  latest.current = { review, language };
  const goTo = (location: CodeLocation): void => {
    const { review: current } = latest.current;
    const view = current.editor.current;
    if (view && current.document?.kind === "source" && current.document.path === location.path) {
      revealEditorLine(view, location.start.row + 1);
      view.focus();
    } else {
      // Jumping away must not let the definition's preview replace the file
      // it was reached from; `gT` returns to it.
      current.keepTab();
      current.openSource(location.path, location.start.row + 1);
    }
  };
  const goToRef = useRef(goTo);
  goToRef.current = goTo;
  useEffect(() => {
    review.languageAction.current = (action: ReviewDocumentAction, view: EditorView) => {
      const { review: current, language: lang } = latest.current;
      if (current.document?.kind !== "source") {
        notify("Open the file (␣VF) to ask about its symbols", "info");
        return;
      }
      if (lang.state === "unavailable" || lang.state === "off") {
        notify("Language intelligence is unavailable for this file", "info");
        return;
      }
      pending.current?.abort();
      const controller = new AbortController();
      pending.current = controller;
      const point = cursorPoint(view);
      if (action.kind === "hover") {
        setHover({ anchor: point.anchor, value: "loading" });
        void lang.hover(point.row, point.column, controller.signal).then(
          (value) => !controller.signal.aborted && setHover({ anchor: point.anchor, value }),
          () => !controller.signal.aborted && setHover({ anchor: point.anchor, value: "error" }),
        );
        return;
      }
      if (action.kind !== "navigate") return;
      const label = NAVIGATION_LABEL[action.to];
      void lang.navigate(point.row, point.column, action.to, controller.signal).then((items) => {
        if (controller.signal.aborted) return;
        if (items === null) notify(`Finding the ${label} needs the language engine's navigation, not available here`, "info");
        else if (items.length === 0) notify(`No ${label} found`, "info");
        else if (items.length === 1) goToRef.current(items[0]!);
        else setLocations({ anchor: point.anchor, items, label });
      }, () => {
        if (!controller.signal.aborted) notify(`Could not find the ${label}`, "warning");
      });
    };
    return () => {
      review.languageAction.current = undefined;
      pending.current?.abort();
    };
  }, [review.languageAction]);
  const back = (): void => {
    setHover(undefined);
    setLocations(undefined);
    requestAnimationFrame(() => latest.current.review.editor.current?.focus());
  };
  return (
    <>
      <Popover
        open={!!hover}
        onClose={back}
        anchorReference="anchorPosition"
        anchorPosition={hover?.anchor ?? { top: 0, left: 0 }}
        slotProps={{ paper: { sx: { maxWidth: 560, maxHeight: 360, overflow: "auto", p: 1.5 }, "data-review-hover": "" } as object }}
      >
        {hover && <HoverBody hover={hover.value} />}
      </Popover>
      <Menu
        open={!!locations}
        onClose={back}
        anchorReference="anchorPosition"
        anchorPosition={locations?.anchor ?? { top: 0, left: 0 }}
        slotProps={{ paper: { "data-review-locations": "" } as object }}
      >
        {locations?.items.map((item, index) => (
          <MenuItem
            key={`${item.path}:${String(item.start.row)}:${String(index)}`}
            onClick={() => {
              setLocations(undefined);
              goToRef.current(item);
            }}
          >
            <ListItemText
              primary={`${item.path}:${String(item.start.row + 1)}`}
              slotProps={{ primary: { sx: { fontSize: "0.78rem", fontFamily: "monospace" } } }}
            />
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}
