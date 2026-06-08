import { useState } from "react";
import {
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  FormControlLabel,
} from "@mui/material";
import { cancelPendingSend, confirmPendingSend, usePendingSend } from "./store";

// Shown when the user manually sends a queued message but the judge can't run
// (no key). The send is never blocked — this just makes the user aware that
// auto-detection of "is the agent waiting?" is off, so the message might land as
// a wrong answer. The always-available LLM-call fallback, with eyes open.
export function ConfirmSendModal(): React.JSX.Element {
  const pending = usePendingSend();
  const [dontAsk, setDontAsk] = useState(false);
  return (
    <Dialog
      open={pending !== undefined}
      onClose={cancelPendingSend}
      maxWidth="xs"
      // Reset the checkbox each time it reopens.
      TransitionProps={{ onExited: (): void => setDontAsk(false) }}
    >
      <DialogTitle>Send without auto-detection?</DialogTitle>
      <DialogContent>
        <DialogContentText>
          No judge key is set, so cowboy can&apos;t tell whether the agent is waiting for
          your reply. Sending now may deliver this message as an answer to a question that
          wasn&apos;t asked.
        </DialogContentText>
        <FormControlLabel
          sx={{ mt: 1 }}
          control={<Checkbox checked={dontAsk} onChange={(e): void => setDontAsk(e.target.checked)} />}
          label="Don't ask again this session"
        />
      </DialogContent>
      <DialogActions>
        <Button onClick={cancelPendingSend} color="inherit">
          Cancel
        </Button>
        <Button onClick={(): void => confirmPendingSend(dontAsk)} variant="contained">
          Send anyway
        </Button>
      </DialogActions>
    </Dialog>
  );
}
