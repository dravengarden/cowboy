import type { ReactNode } from "react";
import { Button, CircularProgress, Stack } from "@mui/material";
import { desktopSize } from "./surface/desktopSize";
import { approvalButtonSx, approvalChoicesInRow } from "./approvalCardStyle";

export interface ApprovalChoice {
  id: string;
  label: string;
  reject: boolean;
  disabled: boolean;
  run: () => void;
}

/** The decision buttons of an interactive card: two short answers share a
 *  row, anything longer stacks. Only the first non-rejecting choice is the
 *  filled default; a broader grant such as "always allow" stays secondary. */
export function ApprovalChoiceButtons({
  choices,
  busy,
  busyChoice,
  keycap,
}: {
  choices: readonly ApprovalChoice[];
  busy: boolean;
  busyChoice: string | null;
  /** Desktop's numbered key hint for a choice. */
  keycap?: (index: number, available: boolean) => ReactNode;
}): React.JSX.Element {
  const firstAllow = choices.findIndex((other) => !other.reject);
  return (
    <Stack
      direction={approvalChoicesInRow(choices.map((choice) => choice.label)) ? "row" : "column"}
      spacing={1}
      sx={{ flexShrink: 0, "& > .MuiButton-root": { flex: 1, minWidth: 0 } }}
    >
      {choices.map((choice, index) => (
        <Button
          key={choice.id}
          fullWidth
          disableElevation
          disabled={busy || choice.disabled}
          variant={!choice.reject && firstAllow === index ? "contained" : "outlined"}
          color={choice.reject ? "error" : "primary"}
          startIcon={busy && busyChoice === choice.id
            ? <CircularProgress size={desktopSize(14)} thickness={5} color="inherit" />
            : undefined}
          endIcon={keycap?.(index, !busy)}
          onClick={choice.run}
          sx={approvalButtonSx}
        >
          {choice.label}
        </Button>
      ))}
    </Stack>
  );
}
