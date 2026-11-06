import { useSyncExternalStore } from "react";
import { Button, CircularProgress, Dialog, DialogActions, DialogContent, DialogContentText, DialogTitle } from "@mui/material";
import { sessionActionSnapshot, subscribeSessionAction } from "./sessionActionWait";

export function SessionActionWaitDialog() {
  const pending = useSyncExternalStore(subscribeSessionAction, sessionActionSnapshot);
  const waiting = pending?.phase === "waiting";
  return (
    <Dialog open={pending !== null} disableRestoreFocus onClose={waiting ? pending?.cancel : undefined}>
      <DialogTitle>{pending?.label}</DialogTitle>
      <DialogContent aria-live="polite">
        <CircularProgress size={24} />
        <DialogContentText>
          {waiting
            ? "Waiting for connection… This action has not been sent. You can cancel safely."
            : "Waiting for confirmation…"}
        </DialogContentText>
      </DialogContent>
      {waiting && <DialogActions><Button onClick={pending?.cancel}>Cancel</Button></DialogActions>}
    </Dialog>
  );
}
