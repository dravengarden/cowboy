import { Alert, Button, FormControlLabel, Stack, Switch, TextField, Typography } from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import {
  AuthApiError,
  authApi,
  type ProductMe,
  type ProductPasskey,
} from "./authApi";
import { createPasskey, passkeysSupported } from "./passkeyBrowser";
import { useProductAuth } from "./ProductAuthGate";

export function ProductPasskeysPanel({
  onMe,
}: {
  onMe?: (me: ProductMe) => void;
}): React.JSX.Element {
  const { me, updateMe } = useProductAuth();
  const [passkeys, setPasskeys] = useState<ProductPasskey[]>([]);
  const [nickname, setNickname] = useState("This device");
  const [enabled, setEnabled] = useState(me.passkey_reauth_enabled !== false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    const body = await authApi.listPasskeys();
    setPasskeys(body.passkeys);
  }, []);

  useEffect(() => {
    void load().catch((err: unknown) => {
      setError(err instanceof AuthApiError ? err.message : "Could not load passkeys");
    });
  }, [load]);

  const add = (): void => {
    if (busy || !passkeysSupported() || nickname.trim() === "") return;
    setBusy(true);
    setError(null);
    void (async () => {
      const ceremony = await authApi.startPasskeyRegister(nickname.trim());
      const credential = await createPasskey(ceremony);
      await authApi.completePasskeyRegister(ceremony.challenge_id, credential);
      setNickname("This device");
      await load();
    })()
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not add a passkey");
      })
      .finally(() => setBusy(false));
  };

  const revoke = (id: string): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void authApi
      .deletePasskey(id)
      .then(load)
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not revoke passkey");
      })
      .finally(() => setBusy(false));
  };

  const toggle = (next: boolean): void => {
    setBusy(true);
    setError(null);
    void authApi
      .setPasskeyReauth(next)
      .then((updated) => {
        setEnabled(updated.passkey_reauth_enabled !== false);
        updateMe(updated);
        onMe?.(updated);
      })
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not save setting");
      })
      .finally(() => setBusy(false));
  };

  return (
    <Stack spacing={1.5}>
      <Typography variant="body2" color="text.secondary">
        After password login, add a Passkey. The web UI locks after 15 minutes
        of viewing and asks for that Passkey. Turn the lock off here.
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      <FormControlLabel
        control={
          <Switch
            checked={enabled}
            disabled={busy}
            onChange={(event) => toggle(event.target.checked)}
          />
        }
        label="Require Passkey after 15 minutes"
      />
      {passkeysSupported() ? (
        <Stack direction="row" spacing={1} alignItems="center">
          <TextField
            size="small"
            label="Passkey name"
            value={nickname}
            onChange={(event) => setNickname(event.target.value)}
            fullWidth
          />
          <Button
            variant="contained"
            disabled={busy || nickname.trim() === ""}
            onClick={add}
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
          <Stack
            key={passkey.id}
            direction="row"
            spacing={1}
            alignItems="center"
            justifyContent="space-between"
          >
            <Typography variant="body2">{passkey.nickname}</Typography>
            <Button
              color="inherit"
              size="small"
              disabled={busy}
              onClick={() => revoke(passkey.id)}
            >
              Revoke
            </Button>
          </Stack>
        ))
      )}
    </Stack>
  );
}
