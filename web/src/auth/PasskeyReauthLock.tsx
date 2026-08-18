import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { AuthApiError, authApi, type ProductMe } from "./authApi";
import { PRODUCT_PASSKEY_IDLE_MS } from "./idleLock";
import { assertPasskey, passkeysSupported } from "./passkeyBrowser";
import { useIdlePasskeyLock } from "./useIdlePasskeyLock";

export function PasskeyReauthLock({
  me,
  onUnlocked,
}: {
  me: ProductMe;
  onUnlocked: (me: ProductMe) => void;
}): React.JSX.Element | null {
  const eligible = me.passkey_reauth_enabled !== false && (me.passkey_count ?? 0) > 0;
  const { locked, unlock } = useIdlePasskeyLock(eligible, PRODUCT_PASSKEY_IDLE_MS);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  if (!locked) return null;

  const confirm = (): void => {
    if (busy || !passkeysSupported()) return;
    setBusy(true);
    setError(null);
    void (async () => {
      const ceremony = await authApi.startPasskeyAssert();
      const credential = await assertPasskey(ceremony);
      const next = await authApi.completePasskeyAssert(ceremony.challenge_id, credential);
      unlock();
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
          This page has been idle for 15 minutes. Use your Passkey to keep
          reading. You can turn this off in Settings.
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
        <Button variant="contained" disabled={busy} onClick={confirm}>
          Continue with Passkey
        </Button>
      </Stack>
    </Box>
  );
}
