import { Alert, Button, Snackbar } from "@mui/material";
import { useState, useSyncExternalStore } from "react";

interface Notice {
  id: number;
  message: string;
  undo?: () => Promise<void>;
}
let notice: Notice | null = null;
let sequence = 0;
const listeners = new Set<() => void>();
function emit(): void {
  for (const listener of listeners) listener();
}
export function documentNotice(
  message: string,
  undo?: () => Promise<void>,
): void {
  notice = { id: ++sequence, message, ...(undo ? { undo } : {}) };
  emit();
}
const subscribe = (listener: () => void): () => void => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const read = (): Notice | null => notice;
export function DocumentNotifications(): React.JSX.Element {
  const current = useSyncExternalStore(subscribe, read, read);
  const [busy, setBusy] = useState(false);
  const close = (): void => {
    if (notice?.id === current?.id) {
      notice = null;
      emit();
    }
  };
  return (
    <Snackbar
      key={current?.id}
      open={current !== null}
      autoHideDuration={busy ? null : 9000}
      onClose={(_, reason) => {
        if (reason !== "clickaway" && !busy) close();
      }}
    >
      <Alert
        severity="info"
        onClose={busy ? undefined : close}
        action={current?.undo
          ? (
            <Button
              disabled={busy}
              color="inherit"
              onClick={() => {
                const undo = current.undo!;
                setBusy(true);
                void undo().then(close).catch((e: unknown) =>
                  documentNotice(
                    e instanceof Error ? e.message : "Could not undo",
                  )
                )
                  .finally(() => setBusy(false));
              }}
            >
              {busy ? "Undoing…" : "Undo"}
            </Button>
          )
          : undefined}
      >
        {current?.message}
      </Alert>
    </Snackbar>
  );
}
