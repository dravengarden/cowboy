import { Close, History } from "@mui/icons-material";
import { Alert, Button, IconButton, Snackbar, Tooltip } from "@mui/material";
import { useState, useSyncExternalStore } from "react";
import { UNDO_NOTICE_MS } from "../undoHistory";
import { undoEntry } from "../undoActions";
import { openUndoHistory } from "../UndoHistorySheet";

interface Notice {
  id: number;
  message: string;
  /** Undo history entry this notice offers. */
  undoId?: string;
}
let notice: Notice | null = null;
let sequence = 0;
const listeners = new Set<() => void>();
function emit(): void {
  for (const listener of listeners) listener();
}
/** A transient message. Undoable changes use recordUndo (undoActions.ts),
 *  which keeps them in Undo history after this notice disappears. */
export function documentNotice(message: string): void {
  showNotice(message);
}
export function showNotice(message: string, undoId?: string): void {
  notice = { id: ++sequence, message, ...(undoId ? { undoId } : {}) };
  emit();
}
const subscribe = (listener: () => void): () => void => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const read = (): Notice | null => notice;
export function DocumentNotifications(
  { mobile = false }: { mobile?: boolean } = {},
): React.JSX.Element {
  const current = useSyncExternalStore(subscribe, read, read);
  const [busy, setBusy] = useState(false);
  const close = (): void => {
    if (notice?.id === current?.id) {
      notice = null;
      emit();
    }
  };
  const undoId = current?.undoId;
  return (
    <Snackbar
      key={current?.id}
      open={current !== null}
      // One duration for every notice; hovering pauses it, and an Undo in
      // flight holds it until the outcome is known.
      autoHideDuration={busy ? null : UNDO_NOTICE_MS}
      onClose={(_, reason) => {
        if (reason !== "clickaway" && !busy) close();
      }}
      anchorOrigin={{
        vertical: "bottom",
        horizontal: mobile ? "center" : "left",
      }}
      // Mobile keeps floating chrome (the Sessions toolbar, the composer
      // actions) on the bottom edge. Lift the notice above it like the
      // app-level error toast instead of covering the controls.
      sx={mobile
        ? { bottom: "calc(env(safe-area-inset-bottom, 0px) + 96px)" }
        : undefined}
    >
      <Alert
        severity="info"
        sx={{
          ...(mobile && { width: "100%" }),
          alignItems: "center",
          // MUI tops the action area with 4px padding and flex-start: center
          // Undo, history and Close on the message instead.
          "& .MuiAlert-message": { py: 0.5 },
          "& .MuiAlert-action": {
            alignItems: "center",
            alignSelf: "center",
            pt: 0,
            mr: -0.5,
            gap: 0.25,
          },
        }}
        onClose={busy ? undefined : close}
        action={undoId
          ? (
            <>
              <Button
                disabled={busy}
                color="inherit"
                onClick={() => {
                  setBusy(true);
                  void undoEntry(undoId).then(close).catch((e: unknown) =>
                    documentNotice(
                      e instanceof Error ? e.message : "Could not undo",
                    )
                  )
                    .finally(() => setBusy(false));
                }}
              >
                {busy ? "Undoing…" : "Undo"}
              </Button>
              <Tooltip title="Undo history">
                <IconButton
                  aria-label="Undo history"
                  color="inherit"
                  size="small"
                  disabled={busy}
                  onClick={() => {
                    close();
                    openUndoHistory();
                  }}
                >
                  <History fontSize="small" />
                </IconButton>
              </Tooltip>
              {/* An Alert with an action drops its own close button; keep a
                  visible way to dismiss the notice before it times out. */}
              <Tooltip title="Dismiss">
                <IconButton
                  aria-label="Dismiss"
                  color="inherit"
                  size="small"
                  disabled={busy}
                  onClick={close}
                >
                  <Close fontSize="small" />
                </IconButton>
              </Tooltip>
            </>
          )
          : undefined}
      >
        {current?.message}
      </Alert>
    </Snackbar>
  );
}
