import Refresh from "@mui/icons-material/Refresh";
import {
  Button,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
} from "@mui/material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { useNetworkActionState } from "./NetworkActionFeedback";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import type { SessionMeta } from "./protocol";
import { reloadSession } from "./sessionReload";

export function SessionReloadDialog({
  session,
  onClose,
}: {
  session: SessionMeta | null | undefined;
  onClose: () => void;
}): React.JSX.Element {
  const action = useNetworkActionState();
  const open = session !== null && session !== undefined;
  const activeTurn = session?.status === "busy" || session?.status === "starting";
  const confirm = (): void => {
    if (!session) return;
    void action.run(async () => {
      await reloadSession(session.id);
      onClose();
    });
  };
  useConfirmEnter(open, confirm);

  return (
    <Dialog
      open={open}
      onClose={(): void => {
        if (!action.pending) onClose();
      }}
      disableRestoreFocus
      maxWidth="xs"
      fullWidth
    >
      <DialogTitle>Reload this session?</DialogTitle>
      <DialogContent>
        <DialogContentText>
          Cowboy will rebuild the agent runtime and load the same session again.
          Conversation history, session ID, title, working directory, queue,
          drafts, and saved agent configuration stay unchanged.
        </DialogContentText>
        {activeTurn && (
          <DialogContentText color="warning.main" sx={{ mt: 1.5 }}>
            The current turn will stop. Output already recorded in the transcript
            remains, but that turn may be incomplete.
          </DialogContentText>
        )}
      </DialogContent>
      <DialogActions>
        <Button color="inherit" onClick={onClose} disabled={action.pending}>
          Cancel
          <Kbd keys="Esc" />
        </Button>
        <Button
          variant="contained"
          startIcon={action.progress ? <CircularProgress size={16} color="inherit" /> : <Refresh />}
          aria-busy={action.pending || undefined}
          disabled={action.pending}
          onClick={confirm}
        >
          Reload
          <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
        </Button>
      </DialogActions>
    </Dialog>
  );
}
