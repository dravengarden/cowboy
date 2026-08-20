import type { PointerEvent as ReactPointerEvent } from "react";
import { Box, Button, CircularProgress } from "@mui/material";
import type { ButtonProps } from "@mui/material";

/**
 * Canonical mobile Cancel / confirm footer.
 *
 * Decisions use explicit text, not ambiguous corner glyphs. The parent sheet
 * owns keyboard-safe positioning; this component only owns the two actions and
 * can retain an input's native focus until the chosen click runs.
 */
export function MobileDecisionActions({
  cancelLabel = "Cancel",
  onCancel,
  cancelDisabled = false,
  confirmLabel,
  onConfirm,
  confirmDisabled = false,
  confirmBusy = false,
  confirmColor = "primary",
  preserveFocus = false,
}: {
  readonly cancelLabel?: string | undefined;
  readonly onCancel: () => void;
  readonly cancelDisabled?: boolean | undefined;
  readonly confirmLabel: string;
  readonly onConfirm: () => void;
  readonly confirmDisabled?: boolean | undefined;
  readonly confirmBusy?: boolean | undefined;
  readonly confirmColor?: ButtonProps["color"];
  /** Keep the current input focused until the selected action click runs. */
  readonly preserveFocus?: boolean | undefined;
}): React.JSX.Element {
  const preserveInput = preserveFocus
    ? (event: ReactPointerEvent): void => event.preventDefault()
    : undefined;
  return (
    <Box
      data-mobile-decision-actions
      sx={{
        width: "100%",
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 1,
      }}
    >
      <Button
        color="inherit"
        disabled={cancelDisabled}
        onPointerDown={preserveInput}
        onClick={onCancel}
        sx={{ minHeight: 44, px: 2, textTransform: "none", fontWeight: 650 }}
      >
        {cancelLabel}
      </Button>
      <Button
        variant="contained"
        color={confirmColor}
        disabled={confirmDisabled || confirmBusy}
        onPointerDown={preserveInput}
        onClick={onConfirm}
        startIcon={confirmBusy
          ? <CircularProgress size={16} color="inherit" />
          : undefined}
        sx={{ minHeight: 44, px: 2.25, textTransform: "none", fontWeight: 650 }}
      >
        {confirmLabel}
      </Button>
    </Box>
  );
}
