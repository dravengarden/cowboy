import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { AuthApiError, authApi, type ProductMe } from "./authApi";
import { assertPasskey, passkeysSupported } from "./passkeyBrowser";

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
    eligible && (serverRequired || (dueAt != null && Date.now() >= dueAt)),
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!eligible || dueAt == null) {
      setLocked(false);
      return;
    }
    const evaluate = (): void => setLocked(serverRequired || Date.now() >= dueAt);
    evaluate();
    const timer = globalThis.setInterval(evaluate, 5_000);
    return () => globalThis.clearInterval(timer);
  }, [dueAt, eligible, serverRequired]);

  if (!locked) return null;

  const confirm = (): void => {
    if (busy || !passkeysSupported()) return;
    setBusy(true);
    setError(null);
    void (async () => {
      const ceremony = await authApi.startPasskeyAssert();
      const credential = await assertPasskey(ceremony);
      const next = await authApi.completePasskeyAssert(ceremony.challenge_id, credential);
      setLocked(false);
      onUnlocked(next);
    })()
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Passkey verification failed");
      })
      .finally(() => setBusy(false));
  };

  return (
    <Box
      sx={{
        position: "fixed",
        inset: 0,
        zIndex: 2000,
        display: "grid",
        placeItems: "center",
        bgcolor: "background.default",
      }}
    >
      <Stack spacing={2} sx={{ width: "100%", maxWidth: 420, p: 3 }}>
        <Typography variant="h5" sx={{ fontWeight: 700 }}>
          Confirm it&apos;s you
        </Typography>
        <Typography color="text.secondary">
          Your configured Passkey refresh is due. Verify now to rotate this
          browser&apos;s session and keep it signed in. You can turn this off or
          change the frequency in Settings.
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
        <Button variant="contained" disabled={busy} onClick={confirm}>
          Continue with Passkey
        </Button>
        <Button color="inherit" disabled={busy} onClick={() => void onSignOut()}>
          Sign out and use a one-day login
        </Button>
      </Stack>
    </Box>
  );
}
