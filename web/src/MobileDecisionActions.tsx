import type { PointerEvent as ReactPointerEvent } from "react";
import { Box, Button, CircularProgress } from "@mui/material";
import type { ButtonProps } from "@mui/material";

/**
 * Canonical mobile Cancel / confirm footer.
 *
 * Decisions use explicit text, not ambiguous corner glyphs. `shelf` paints the
 * New-session hairline strip so a reserved DetentSheet footer does not float
 * over the form. The parent still owns overlay-vs-reserved positioning.
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
  shelf = false,
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
  /** Bleed a solid Cancel/confirm strip to the sheet edges. */
  readonly shelf?: boolean | undefined;
}): React.JSX.Element {
  const preserveInput = preserveFocus
    ? (event: ReactPointerEvent): void => event.preventDefault()
    : undefined;
  const actions = (
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
  if (!shelf) return actions;
  return (
    <Box
      data-mobile-decision-footer-shelf
      sx={{
        // DetentSheet owns a 16px footer gutter. Bleed this strip back to the
        // sheet edges so it reads as bottom chrome, not a card in the form.
        width: "calc(100% + 32px)",
        mx: -2,
        px: 2,
        pt: 0.75,
        bgcolor: "background.default",
        borderTop: 1,
        borderColor: "divider",
      }}
    >
      {actions}
    </Box>
  );
}
