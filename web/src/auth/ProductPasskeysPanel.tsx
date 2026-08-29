import {
  Alert,
  Button,
  FormControl,
  FormControlLabel,
  InputLabel,
  MenuItem,
  Select,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import {
  AuthApiError,
  authApi,
  type ProductMe,
  type ProductPasskey,
} from "./authApi";
import { assertPasskey, createPasskey, passkeysSupported } from "./passkeyBrowser";
import { useProductAuth } from "./ProductAuthGate";

const REFRESH_INTERVALS = [
  { label: "Every day", value: 24 * 60 * 60 * 1_000 },
  { label: "Every 7 days", value: 7 * 24 * 60 * 60 * 1_000 },
  { label: "Every 14 days", value: 14 * 24 * 60 * 60 * 1_000 },
] as const;

export function ProductPasskeysPanel({
  onMe,
}: {
  onMe?: (me: ProductMe) => void;
}): React.JSX.Element {
  const { me, updateMe } = useProductAuth();
  const [passkeys, setPasskeys] = useState<ProductPasskey[]>([]);
  const [nickname, setNickname] = useState("This device");
  const [enabled, setEnabled] = useState(me.passkey_reauth_enabled === true);
  const [reauthAfterMs, setReauthAfterMs] = useState(
    me.passkey_reauth_after_ms ?? REFRESH_INTERVALS[1].value,
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    const body = await authApi.listPasskeys();
    setPasskeys(body.passkeys);
    setReauthAfterMs(body.reauth_after_ms);
  }, []);

  useEffect(() => {
    if (me.auth_enabled === false) return;
    void load().catch((err: unknown) => {
      setError(err instanceof AuthApiError ? err.message : "Could not load passkeys");
    });
  }, [load, me.auth_enabled]);

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
      updateMe(await authApi.me());
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
      .then(async () => {
        await load();
        const updated = await authApi.me();
        setEnabled(updated.passkey_reauth_enabled === true);
        updateMe(updated);
        onMe?.(updated);
      })
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not revoke passkey");
      })
      .finally(() => setBusy(false));
  };

  const toggle = (next: boolean): void => {
    setBusy(true);
    setError(null);
    void (async () => {
      let updated = await authApi.setPasskeyReauth(next, reauthAfterMs);
      if (next) {
        try {
          const ceremony = await authApi.startPasskeyAssert();
          const credential = await assertPasskey(ceremony);
          updated = await authApi.completePasskeyAssert(ceremony.challenge_id, credential);
        } catch (error) {
          await authApi.setPasskeyReauth(false, reauthAfterMs).catch(() => undefined);
          throw error;
        }
      }
      return updated;
    })()
      .then((updated) => {
        setEnabled(updated.passkey_reauth_enabled === true);
        updateMe(updated);
        onMe?.(updated);
      })
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not save setting");
      })
      .finally(() => setBusy(false));
  };

  const changeInterval = (next: number): void => {
    setBusy(true);
    setError(null);
    void authApi
      .setPasskeyReauth(enabled, next)
      .then((updated) => {
        setReauthAfterMs(updated.passkey_reauth_after_ms ?? next);
        updateMe(updated);
        onMe?.(updated);
      })
      .catch((err: unknown) => {
        setError(err instanceof AuthApiError ? err.message : "Could not save setting");
      })
      .finally(() => setBusy(false));
  };

  if (me.auth_enabled === false) return <></>;

  return (
    <Stack spacing={1.5}>
      <Typography variant="body2" color="text.secondary">
        Password and Cardea sign-ins last one day. Passkey refresh is optional
        and off by default. A successful Passkey rotates this browser&apos;s
        session and extends it for up to 30 days. Turning it on verifies your
        Passkey immediately.
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      <FormControlLabel
        control={
          <Switch
            checked={enabled}
            disabled={busy || passkeys.length === 0}
            onChange={(event) => toggle(event.target.checked)}
          />
        }
        label="Keep this session signed in with Passkey refresh"
      />
      <FormControl size="small" disabled={busy || passkeys.length === 0}>
        <InputLabel id="passkey-refresh-interval-label">Refresh frequency</InputLabel>
        <Select
          labelId="passkey-refresh-interval-label"
          label="Refresh frequency"
          value={reauthAfterMs}
          onChange={(event) => changeInterval(Number(event.target.value))}
        >
          {REFRESH_INTERVALS.map((option) => (
            <MenuItem key={option.value} value={option.value}>{option.label}</MenuItem>
          ))}
        </Select>
      </FormControl>
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
          No passkeys yet. Add one, then turn on session refresh when you want it.
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
