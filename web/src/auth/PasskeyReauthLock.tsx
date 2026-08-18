import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import { AuthApiError, authApi, type ProductMe } from "./authApi";
import { assertPasskey, passkeysSupported } from "./passkeyBrowser";

export function PasskeyReauthLock({
  me,
  onUnlocked,
}: {
  me: ProductMe;
  onUnlocked: (me: ProductMe) => void;
}): React.JSX.Element | null {
  const [locked, setLocked] = useState(me.passkey_reauth_required === true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async (): Promise<void> => {
    const next = await authApi.me();
    setLocked(next.passkey_reauth_required === true);
    if (next.passkey_reauth_required !== true) onUnlocked(next);
  }, [onUnlocked]);

  useEffect(() => {
    setLocked(me.passkey_reauth_required === true);
  }, [me.passkey_reauth_required]);

  useEffect(() => {
    const timer = globalThis.setInterval(() => {
      void refresh().catch(() => {
        // Keep the current lock; the next interval retries.
      });
    }, 30_000);
    const onVisible = (): void => {
      if (document.visibilityState === "visible") {
        void refresh().catch(() => undefined);
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      globalThis.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refresh]);

  if (!locked) return null;

  const unlock = (): void => {
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
          This view has been open for 15 minutes. Use your Passkey to keep
          reading. You can turn this off in Settings.
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
        <Button variant="contained" disabled={busy} onClick={unlock}>
          Continue with Passkey
        </Button>
      </Stack>
    </Box>
  );
}
