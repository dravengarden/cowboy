import { Button, Typography } from "@mui/material";
import type { SessionMeta } from "./protocol";
import { sessionIsWorking } from "./trashModel";
import { useNavbarAtBottom } from "./navbarSettings";
import { Sheet } from "./Sheet";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";

/** Desktop always confirms; background work requires permanent deletion. */
export function DeleteSessionShell({
  session,
  onClose,
  onStopAndTrash,
  onDeletePermanently,
}: {
  session: SessionMeta | null;
  onClose: () => void;
  onStopAndTrash: () => void;
  onDeletePermanently: () => void;
}): React.JSX.Element | null {
  // Hooks before the early return (rules of hooks).
  const navbarAtBottom = useNavbarAtBottom();
  const turnRunning = session?.status === "busy";
  const permanent = session !== null && !turnRunning &&
    sessionIsWorking(session);
  useConfirmEnter(
    session !== null,
    permanent ? onDeletePermanently : onStopAndTrash,
  );
  if (!session) return null;
  return (
    <Sheet
      forceSheet={navbarAtBottom}
      open
      onClose={onClose}
      title={permanent
        ? "Delete this session permanently?"
        : turnRunning
        ? "Stop this session and move it to Trash?"
        : "Move this session to Trash?"}
      mobileDismiss="none"
      actions={
        <>
          <Button onClick={onClose} color="inherit">
            Cancel
            <Kbd keys="Esc" />
          </Button>
          {!permanent
            ? (
              <Button
                onClick={onStopAndTrash}
                color="error"
                variant="contained"
              >
                {turnRunning ? "Stop and move to Trash" : "Move to Trash"}
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </Button>
            )
            : (
              <Button
                onClick={onDeletePermanently}
                color="error"
                variant="contained"
              >
                Delete permanently
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </Button>
            )}
        </>
      }
    >
      <Typography variant="body2" color="text.secondary">
        {turnRunning
          ? "Its current turn is cancelled. The session then moves to Trash, where it can be restored with its transcript and worktree."
          : permanent
          ? "It is still running background work, which cannot wait in Trash. Deleting it now stops that work and removes the transcript. This cannot be undone."
          : "The session moves to Trash, where it can be restored with its transcript and worktree."}
      </Typography>
    </Sheet>
  );
}
