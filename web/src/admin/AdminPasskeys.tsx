import { Alert, Button, FormControlLabel, Paper, Stack, Switch, TextField, Typography } from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import {
  assertPasskey,
  createPasskey,
  passkeysSupported,
} from "../auth/passkeyBrowser";
import {
  adminApi,
  type AdminAuthStatus,
  type AdminPasskey,
} from "./adminApi";

export function AdminPasskeysCard({
  auth,
  onAuth,
}: {
  auth: AdminAuthStatus;
  onAuth: (auth: AdminAuthStatus) => void;
}): React.JSX.Element {
  const [passkeys, setPasskeys] = useState<AdminPasskey[]>([]);
  const [nickname, setNickname] = useState("This device");
  const [enabled, setEnabled] = useState(auth.passkey_reauth_enabled !== false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    const body = await adminApi.listPasskeys();
    setPasskeys(body.passkeys);
  }, []);

  useEffect(() => {
    void load().catch((err: Error) => setError(err.message));
  }, [load]);

  return (
    <Paper sx={{ p: 2 }}>
      <Stack spacing={1.5}>
        <Typography variant="h6">Passkeys</Typography>
        <Typography color="text.secondary">
          After password login, add a Passkey. The admin console locks after 15
          minutes of viewing when a Passkey exists. Turn the lock off here.
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
        <FormControlLabel
          control={
            <Switch
              checked={enabled}
              disabled={busy}
              onChange={(event) => {
                setBusy(true);
                void adminApi.setPasskeyReauth(event.target.checked).then((next) => {
                  setEnabled(next.passkey_reauth_enabled !== false);
                  onAuth(next);
                }).catch((err: Error) => setError(err.message)).finally(() => setBusy(false));
              }}
            />
          }
          label="Require Passkey after 15 minutes"
        />
        {passkeysSupported() ? (
          <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
            <TextField
              size="small"
              label="Passkey name"
              value={nickname}
              onChange={(event) => setNickname(event.target.value)}
            />
            <Button
              variant="outlined"
              disabled={busy || nickname.trim() === ""}
              onClick={() => {
                setBusy(true);
                setError(null);
                void (async () => {
                  const ceremony = await adminApi.startPasskeyRegister(nickname.trim());
                  const credential = await createPasskey(ceremony);
                  await adminApi.completePasskeyRegister(ceremony.challenge_id, credential);
                  setNickname("This device");
                  await load();
                })().catch((err: Error) => setError(err.message)).finally(() => setBusy(false));
              }}
            >
              Add
            </Button>
          </Stack>
        ) : (
          <Alert severity="info">This browser cannot create a Passkey.</Alert>
        )}
        {passkeys.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No passkeys yet. The viewing lock stays off until you add one.
          </Typography>
        ) : (
          passkeys.map((passkey) => (
            <Stack key={passkey.id} direction="row" spacing={1} alignItems="center" justifyContent="space-between">
              <Typography variant="body2">{passkey.nickname}</Typography>
              <Button
                size="small"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  void adminApi.deletePasskey(passkey.id).then(load).catch((err: Error) => setError(err.message)).finally(() => setBusy(false));
                }}
              >
                Revoke
              </Button>
            </Stack>
          ))
        )}
      </Stack>
    </Paper>
  );
}

export function AdminPasskeyLock({
  auth,
  onAuth,
}: {
  auth: AdminAuthStatus;
  onAuth: (auth: AdminAuthStatus) => void;
}): React.JSX.Element | null {
  const [locked, setLocked] = useState(auth.passkey_reauth_required === true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async (): Promise<void> => {
    const next = await adminApi.auth();
    setLocked(next.authenticated && next.passkey_reauth_required === true);
    onAuth(next);
  }, [onAuth]);

  useEffect(() => {
    setLocked(auth.passkey_reauth_required === true);
  }, [auth.passkey_reauth_required]);

  useEffect(() => {
    const timer = globalThis.setInterval(() => {
      void refresh().catch(() => undefined);
    }, 30_000);
    return () => globalThis.clearInterval(timer);
  }, [refresh]);

  if (!locked) return null;

  return (
    <Stack
      spacing={2}
      sx={{
        position: "fixed",
        inset: 0,
        zIndex: 2000,
        display: "grid",
        placeItems: "center",
        bgcolor: "background.default",
        p: 3,
      }}
    >
      <Stack spacing={2} sx={{ maxWidth: 420 }}>
        <Typography variant="h5">Confirm it&apos;s you</Typography>
        <Typography color="text.secondary">
          Admin has been open for 15 minutes. Use your Passkey to continue.
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
        <Button
          variant="contained"
          disabled={busy}
          onClick={() => {
            if (busy || !passkeysSupported()) return;
            setBusy(true);
            setError(null);
            void (async () => {
              const ceremony = await adminApi.startPasskeyAssert();
              const credential = await assertPasskey(ceremony);
              const next = await adminApi.completePasskeyAssert(ceremony.challenge_id, credential);
              setLocked(false);
              onAuth(next);
            })().catch((err: Error) => setError(err.message)).finally(() => setBusy(false));
          }}
        >
          Continue with Passkey
        </Button>
      </Stack>
    </Stack>
  );
}
