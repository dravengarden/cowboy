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
  authApi,
  AuthApiError,
  type ProductMe,
  type ProductPasskey,
} from "./authApi";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  registerPasskey,
  verifyPasskey,
} from "./passkeyFlow";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";

const REFRESH_INTERVALS = [
  { label: "Every 4 hours", value: 4 * 60 * 60 * 1_000 },
  { label: "Every 8 hours", value: 8 * 60 * 60 * 1_000 },
  { label: "Every 12 hours", value: 12 * 60 * 60 * 1_000 },
  { label: "Every day", value: 24 * 60 * 60 * 1_000 },
  { label: "Every 3 days · Default", value: 3 * 24 * 60 * 60 * 1_000 },
  { label: "Every 7 days", value: 7 * 24 * 60 * 60 * 1_000 },
  { label: "Every 14 days", value: 14 * 24 * 60 * 60 * 1_000 },
] as const;
const DEFAULT_REAUTH_INTERVAL_MS = 3 * 24 * 60 * 60 * 1_000;

export function ProductPasskeysPanel({
  onMe,
}: {
  onMe?: (me: ProductMe) => void;
}): React.JSX.Element {
  const { me, passkeys: policy, reauthenticate, updateMe } = useProductAuth();
  const [passkeys, setPasskeys] = useState<ProductPasskey[]>([]);
  const [nickname, setNickname] = useState("");
  const [enabled, setEnabled] = useState(me.passkey_reauth_enabled === true);
  const [reauthAfterMs, setReauthAfterMs] = useState(
    me.passkey_reauth_after_ms ?? DEFAULT_REAUTH_INTERVAL_MS,
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    const body = await authApi.listPasskeys();
    setPasskeys(body.passkeys);
    setReauthAfterMs(body.reauth_after_ms);
  }, []);

  useEffect(() => {
    if (me.auth_enabled === false || policy?.enabled === false) return;
    void load().catch((err: unknown) => {
      setError(
        err instanceof AuthApiError ? err.message : "Could not load passkeys",
      );
    });
  }, [load, me.auth_enabled, policy?.enabled]);

  const add = (): void => {
    if (busy || !passkeyFlowSupported() || nickname.trim() === "") return;
    setBusy(true);
    setError(null);
    void (async () => {
      await retryWithRecentProductAuth(
        () => registerPasskey(nickname.trim()),
        reauthenticate,
      );
      setNickname("");
      await load();
      updateMe(await authApi.me());
    })()
      .catch((err: unknown) => {
        setError(passkeyErrorMessage(err, "Could not add a passkey"));
      })
      .finally(() => setBusy(false));
  };

  const revoke = (id: string): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    void retryWithRecentProductAuth(
      () => authApi.deletePasskey(id),
      reauthenticate,
    )
      .then(async () => {
        await load();
        const updated = await authApi.me();
        setEnabled(updated.passkey_reauth_enabled === true);
        updateMe(updated);
        onMe?.(updated);
      })
      .catch((err: unknown) => {
        setError(
          err instanceof AuthApiError
            ? err.message
            : "Could not revoke passkey",
        );
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
          updated = await verifyPasskey();
        } catch (error) {
          await authApi.setPasskeyReauth(false, reauthAfterMs).catch(() =>
            undefined
          );
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
        if (passkeyFlowCancelled(err)) return;
        setError(passkeyErrorMessage(err, "Could not save setting"));
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
        setError(
          err instanceof AuthApiError ? err.message : "Could not save setting",
        );
      })
      .finally(() => setBusy(false));
  };

  if (me.auth_enabled === false) return <></>;
  if (policy?.enabled === false) {
    return (
      <Alert severity="info">
        Passkeys are disabled by this Cowboy Service.
      </Alert>
    );
  }

  const refreshEnabled = policy?.session_refresh_enabled !== false;

  return (
    <Stack spacing={1.5}>
      <Typography variant="body2" color="text.secondary">
        Password and external sign-ins last one day. Periodic Passkey checks are
        optional and off by default. When one is due, Cowboy hides this screen
        until you choose to unlock it; running agents continue in the
        background. A successful check rotates this browser&apos;s session and
        extends it for up to 30 days.
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      {refreshEnabled
        ? (
          <>
            <FormControlLabel
              control={
                <Switch
                  checked={enabled}
                  disabled={busy || passkeys.length === 0}
                  onChange={(event) => toggle(event.target.checked)}
                />
              }
              label="Require Passkey periodically"
            />
            <FormControl size="small" disabled={busy || passkeys.length === 0}>
              <InputLabel id="passkey-refresh-interval-label">
                Verification frequency
              </InputLabel>
              <Select
                labelId="passkey-refresh-interval-label"
                label="Verification frequency"
                value={reauthAfterMs}
                onChange={(event) => changeInterval(Number(event.target.value))}
              >
                {REFRESH_INTERVALS.map((option) => (
                  <MenuItem key={option.value} value={option.value}>
                    {option.label}
                  </MenuItem>
                ))}
              </Select>
            </FormControl>
          </>
        )
        : (
          <Alert severity="info">
            Periodic Passkey verification is disabled by this Cowboy Service.
          </Alert>
        )}
      {passkeyFlowSupported()
        ? (
          <Stack direction="row" spacing={1} alignItems="center">
            <TextField
              size="small"
              label="Passkey name"
              value={nickname}
              onChange={(event) => setNickname(event.target.value)}
              slotProps={{ htmlInput: { maxLength: 64 } }}
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
        )
        : <Alert severity="info">This browser cannot create a Passkey.</Alert>}
      {passkeys.length === 0
        ? (
          <Typography variant="body2" color="text.secondary">
            No passkeys yet. Add one, then turn on periodic verification when
            you want it. The name is required so you can identify and revoke the
            right device later.
          </Typography>
        )
        : (
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
