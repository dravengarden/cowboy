import { Box } from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import { DESKTOP_SPLITTER_ADJUST_EVENT, splitterAdjustment } from "../desktopSplitterKeyboard";
import { DesktopSplitterHint } from "../DesktopSplitterHint";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";

const STORAGE_KEY = "cowboy.desktop.reviewNavigatorWidth.v1";
export const REVIEW_NAVIGATOR_MIN = 220;
const NAVIGATOR_MAX = 560;
const NAVIGATOR_DEFAULT = 300;
/** The Document keeps at least this much beside the Navigator. */
const DOCUMENT_MIN = 360;

/** A Navigator width that leaves the Document its floor in `available`. */
export function clampNavigatorWidth(width: number, available: number): number {
  const max = Math.max(REVIEW_NAVIGATOR_MIN, Math.min(NAVIGATOR_MAX, available - DOCUMENT_MIN));
  return Math.round(Math.min(max, Math.max(REVIEW_NAVIGATOR_MIN, width)));
}

function stored(): number {
  try {
    const value = Number(globalThis.localStorage?.getItem(STORAGE_KEY));
    return Number.isFinite(value) && value > 0 ? value : NAVIGATOR_DEFAULT;
  } catch {
    return NAVIGATOR_DEFAULT;
  }
}

/** The Navigator's persisted width, resized by dragging or Resize mode. */
export function useNavigatorWidth(available: number): [number, (width: number) => void] {
  const [width, setWidth] = useState(stored);
  const set = useCallback((next: number): void => {
    setWidth(next);
    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, String(Math.round(next)));
    } catch {
      // The width then lasts for this page only.
    }
  }, []);
  useEffect(() => {
    const onKeyboard = (event: Event): void => {
      const adjustment = splitterAdjustment(event);
      if (adjustment?.splitter !== "review-navigator") return;
      set(clampNavigatorWidth(width + adjustment.delta, available));
    };
    globalThis.addEventListener(DESKTOP_SPLITTER_ADJUST_EVENT, onKeyboard);
    return () => globalThis.removeEventListener(DESKTOP_SPLITTER_ADJUST_EVENT, onKeyboard);
  }, [available, set, width]);
  return [available > 0 ? clampNavigatorWidth(width, available) : width, set];
}

/** The Navigator's edge: drag it, or select it in Resize mode (`␣WR`). */
export function ReviewNavigatorSplitter({
  width,
  available,
  onWidth,
}: {
  width: number;
  available: number;
  onWidth: (width: number) => void;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const selected = workspace.selectedSplitter === "review-navigator";
  const [dragging, setDragging] = useState(false);
  return (
    <Box
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the Review Navigator"
      aria-valuenow={width}
      data-desktop-splitter="review-navigator"
      data-desktop-splitter-selected={selected ? "true" : undefined}
      tabIndex={-1}
      onPointerDown={(event) => {
        const start = event.clientX;
        const from = width;
        event.currentTarget.setPointerCapture(event.pointerId);
        setDragging(true);
        const move = (next: PointerEvent): void => onWidth(clampNavigatorWidth(from + next.clientX - start, available));
        const up = (): void => {
          setDragging(false);
          globalThis.removeEventListener("pointermove", move);
          globalThis.removeEventListener("pointerup", up);
        };
        globalThis.addEventListener("pointermove", move);
        globalThis.addEventListener("pointerup", up);
      }}
      sx={{
        flex: "0 0 auto",
        alignSelf: "stretch",
        width: "1px",
        bgcolor: dragging || selected ? "primary.main" : "divider",
        position: "relative",
        cursor: "col-resize",
        touchAction: "none",
        zIndex: 3,
        "&::after": { content: '""', position: "absolute", top: 0, bottom: 0, left: "-5px", right: "-5px" },
        "&:hover": { bgcolor: "primary.main" },
        "&:focus": { outline: "none" },
      }}
    >
      {selected && <DesktopSplitterHint />}
    </Box>
  );
}
