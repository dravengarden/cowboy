import { type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Box } from "@mui/material";
import { useKeyboardOpen } from "./keyboardInset";

const Z = 1260;

/**
 * Thumb-reachable action dock for cover sheets.
 *
 * DetentSheet footers live inside the sheet's transformed box and still
 * pad for the home indicator after PWA resizes-content zeroes --kb-inset.
 * That lifts Cancel/Create into the form and leaves a dead band above the
 * iOS accessory. This dock is a separate layer, pinned to the same
 * --vv-offset/--vv-height box the cover uses, so the islands sit on the
 * visual viewport's bottom edge.
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
    <Box
      data-mobile-thumb-dock="true"
      sx={{
        position: "fixed",
        top: "var(--vv-offset, 0px)",
        left: 0,
        right: 0,
        height: "var(--vv-height, 100dvh)",
        zIndex: Z,
        pointerEvents: "none",
        display: "flex",
        alignItems: "flex-end",
        justifyContent: "space-between",
        px: 2,
        pb: keyboardOpen
          ? "10px"
          : "max(12px, env(safe-area-inset-bottom, 0px))",
      }}
    >
      <Box
        sx={{
          pointerEvents: "auto",
          flex: "0 0 auto",
          "& > *": { width: "auto" },
        }}
      >
        {left}
      </Box>
      <Box
        sx={{
          pointerEvents: "auto",
          flex: "0 0 auto",
          ml: "auto",
          "& > *": { width: "auto" },
        }}
      >
        {right}
      </Box>
    </Box>
  );
  return globalThis.document?.body
    ? createPortal(dock, globalThis.document.body)
    : dock;
}
