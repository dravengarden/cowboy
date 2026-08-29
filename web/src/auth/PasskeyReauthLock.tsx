import { Alert, Button, Modal, Paper, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { type ProductMe } from "./authApi";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  verifyPasskey,
} from "./passkeyFlow";
import {
  passkeyReauthDue,
  passkeyReauthTimerDelay,
} from "./passkeyReauthSchedule";

export function PasskeyReauthLock({
  me,
  onUnlocked,
  onSignOut,
}: {
  me: ProductMe;
  onUnlocked: (me: ProductMe) => void;
  onSignOut: () => Promise<void>;
}): React.JSX.Element | null {
  const eligible = me.passkey_reauth_enabled === true && (me.passkey_count ?? 0) > 0;
  const dueAt = me.passkey_reauth_due_at_ms ?? null;
  const serverRequired = me.passkey_reauth_required === true;
  const [locked, setLocked] = useState(
    passkeyReauthDue(eligible, serverRequired, dueAt, Date.now()),
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let timer: number | undefined;
    const arm = (): void => {
      if (timer != null) globalThis.clearTimeout(timer);
      const now = Date.now();
      const due = passkeyReauthDue(eligible, serverRequired, dueAt, now);
      setLocked(due);
      const delay = passkeyReauthTimerDelay(
        eligible,
        serverRequired,
        dueAt,
        now,
      );
      if (!due && delay != null) timer = globalThis.setTimeout(arm, delay);
    };
    arm();
    globalThis.addEventListener("focus", arm);
    globalThis.document.addEventListener("visibilitychange", arm);
    return () => {
      if (timer != null) globalThis.clearTimeout(timer);
      globalThis.removeEventListener("focus", arm);
      globalThis.document.removeEventListener("visibilitychange", arm);
    };
  }, [dueAt, eligible, serverRequired]);

  if (!locked) return null;

  const confirm = (): void => {
    if (busy || !passkeyFlowSupported()) return;
    setBusy(true);
    setError(null);
    void (async () => {
      const next = await verifyPasskey();
      setLocked(false);
      onUnlocked(next);
    })()
      .catch((err: unknown) => {
        if (passkeyFlowCancelled(err)) return;
        setError(passkeyErrorMessage(err, "Passkey verification failed"));
      })
      .finally(() => setBusy(false));
  };

  return (
    <Modal
      open
      disableEscapeKeyDown
      aria-labelledby="passkey-lock-title"
      aria-describedby="passkey-lock-description"
      slotProps={{
        backdrop: {
          sx: {
            bgcolor: (theme) =>
              theme.palette.mode === "dark"
                ? "rgba(7, 5, 13, 0.88)"
                : "rgba(247, 246, 250, 0.9)",
            backdropFilter: "blur(24px) saturate(65%)",
            WebkitBackdropFilter: "blur(24px) saturate(65%)",
          },
        },
      }}
    >
      <Paper
        elevation={20}
        sx={{
          position: "absolute",
          top: "50%",
          left: "50%",
          transform: "translate(-50%, -50%)",
          width: "calc(100% - 32px)",
          maxWidth: 440,
          border: 1,
          borderColor: "divider",
          borderRadius: 3,
          outline: 0,
          p: { xs: 2.5, sm: 3 },
        }}
      >
        <Stack spacing={2}>
          <Typography id="passkey-lock-title" variant="h5" sx={{ fontWeight: 700 }}>
            Cowboy is locked
          </Typography>
          <Typography id="passkey-lock-description" color="text.secondary">
            Your scheduled Passkey check is due. Cowboy has hidden this screen
            until you unlock it; running agents continue in the background.
          </Typography>
          {error && <Alert severity="error">{error}</Alert>}
          <Button
            autoFocus
            variant="contained"
            disabled={busy || !passkeyFlowSupported()}
            onClick={confirm}
          >
            Unlock with Passkey
          </Button>
          <Button
            color="inherit"
            disabled={busy}
            onClick={() => void onSignOut()}
          >
            Sign out and use a one-day login
          </Button>
        </Stack>
      </Paper>
    </Modal>
  );
}
