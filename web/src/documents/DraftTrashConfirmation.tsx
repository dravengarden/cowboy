import { Button, Typography } from "@mui/material";
import { ConfirmSheet } from "../Sheet";
import { Kbd, useConfirmEnter } from "../Kbd";
import { ENTER_LABEL, MOD_LABEL } from "../platform";

export function DraftTrashConfirmation(
  { title, busy, onClose, onConfirm, children }: {
    title: string;
    busy: boolean;
    onClose: () => void;
    onConfirm: () => void;
    children: React.ReactNode;
  },
): React.JSX.Element {
  useConfirmEnter(!busy, onConfirm);
  return (
    <ConfirmSheet
      open
      title={`Move “${title}” to Trash?`}
      onClose={busy ? () => {} : onClose}
      actions={
        <>
          <Button color="inherit" disabled={busy} onClick={onClose}>
            Cancel<Kbd keys="Esc" />
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={busy}
            onClick={onConfirm}
          >
            Move to Trash<Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </>
      }
    >
      <Typography>{children}</Typography>
    </ConfirmSheet>
  );
}
