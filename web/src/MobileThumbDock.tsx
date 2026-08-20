import { type ReactNode } from "react";
import { createPortal } from "react-dom";
import CheckIcon from "@mui/icons-material/Check";
import CloseIcon from "@mui/icons-material/Close";
import { Box, CircularProgress } from "@mui/material";
import { MobileSheetActionGroup } from "./_shell";
import { useKeyboardOpen } from "./keyboardInset";

/** Trailing body pad so inner sheet scroll can clear the corner islands. */
export const SHEET_THUMB_CLEARANCE = "88px";

const Z = 1260;

function islandPin(side: "left" | "right", keyboardOpen: boolean): object {
  const inset = keyboardOpen
    ? "10px"
    : "max(12px, env(safe-area-inset-bottom, 0px))";
  return {
    position: "fixed",
    top: `calc(var(--vv-offset, 0px) + var(--vv-height, 100dvh) - ${inset})`,
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

/**
 * Cancel / confirm islands for mobile sheets. Same geometry as New session:
 * pinned to the visual-viewport corners so the sheet body can scroll behind
 * them, including while the software keyboard is up.
 */
export function MobileDecisionDock({
  open,
  cancelLabel = "Cancel",
  onCancel,
  cancelDisabled = false,
  confirmLabel,
  onConfirm,
  confirmDisabled = false,
  confirmBusy = false,
}: {
  readonly open: boolean;
  readonly cancelLabel?: string | undefined;
  readonly onCancel: () => void;
  readonly cancelDisabled?: boolean | undefined;
  readonly confirmLabel: string;
  readonly onConfirm: () => void;
  readonly confirmDisabled?: boolean | undefined;
  readonly confirmBusy?: boolean | undefined;
}): ReactNode {
  return (
    <MobileThumbDock
      open={open}
      left={
        <MobileSheetActionGroup
          actions={[{
            key: "cancel",
            label: cancelLabel,
            disabled: cancelDisabled,
            onPress: onCancel,
            icon: (
              <CloseIcon
                aria-hidden
                fontSize="small"
                sx={{ transform: "translate(-0.75px, -0.5px)" }}
              />
            ),
          }]}
        />
      }
      right={
        <MobileSheetActionGroup
          actions={[{
            key: "confirm",
            label: confirmLabel,
            disabled: confirmDisabled || confirmBusy,
            onPress: onConfirm,
            icon: confirmBusy
              ? <CircularProgress size={18} color="inherit" />
              : <CheckIcon fontSize="small" />,
          }]}
        />
      }
    />
  );
}
