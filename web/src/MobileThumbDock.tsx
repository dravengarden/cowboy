import { type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Box } from "@mui/material";
import { useKeyboardOpen } from "./keyboardInset";

const Z = 1260;

function islandPin(side: "left" | "right", keyboardOpen: boolean): object {
  const inset = keyboardOpen
    ? "10px"
    : "max(12px, env(safe-area-inset-bottom, 0px))";
  return {
    position: "fixed",
    top:
      `calc(var(--vv-offset, 0px) + var(--vv-height, 100dvh) - ${inset})`,
    transform: "translateY(-100%)",
    [side]: 16,
    zIndex: Z,
    pointerEvents: "auto",
    "& > *": { width: "auto" },
  };
}

/**
 * Thumb-reachable action islands for cover sheets.
 *
 * Do not wrap these in a full-viewport overlay. iOS WebKit still hits
 * `pointer-events: none` flex layers and freezes the page underneath.
 * Pin each island to the visual-viewport bottom corners instead.
 */
export function MobileThumbDock({
  open,
  left,
  right,
}: {
  readonly open: boolean;
  readonly left?: ReactNode;
  readonly right?: ReactNode;
}): ReactNode {
  const keyboardOpen = useKeyboardOpen();
  if (!open || (left == null && right == null)) return null;
  const dock = (
    <>
      {left == null ? null : (
        <Box
          data-mobile-thumb-dock="left"
          sx={islandPin("left", keyboardOpen)}
        >
          {left}
        </Box>
      )}
      {right == null ? null : (
        <Box
          data-mobile-thumb-dock="right"
          sx={islandPin("right", keyboardOpen)}
        >
          {right}
        </Box>
      )}
    </>
  );
  return globalThis.document?.body
    ? createPortal(dock, globalThis.document.body)
    : dock;
}
