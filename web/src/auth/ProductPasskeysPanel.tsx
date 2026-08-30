import AddRounded from "@mui/icons-material/AddRounded";
import KeyRounded from "@mui/icons-material/KeyRounded";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
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
const CARD_SX = {
  border: 1,
  borderColor: "divider",
  borderRadius: 3,
  p: 2,
} as const;

type ListState = "loading" | "ready" | "error";
type Notice = { severity: "info" | "success"; message: string };

function passkeyDate(createdAtMs: number): string {
  try {
    return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(
      new Date(createdAtMs),
    );
  } catch {
    return "Recently added";
  }
}

export function ProductPasskeysPanel({
  onMe,
}: {
  onMe?: (me: ProductMe) => void;
}): React.JSX.Element {
  const { me, passkeys: policy, reauthenticate, updateMe } = useProductAuth();
  const [passkeys, setPasskeys] = useState<ProductPasskey[]>([]);
  const [listState, setListState] = useState<ListState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [nickname, setNickname] = useState("");
  const [showAddAnother, setShowAddAnother] = useState(false);
  const [enabled, setEnabled] = useState(me.passkey_reauth_enabled === true);
  const [reauthAfterMs, setReauthAfterMs] = useState(
    me.passkey_reauth_after_ms ?? DEFAULT_REAUTH_INTERVAL_MS,
  );
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    setListState("loading");
    setLoadError(null);
    try {
      const body = await authApi.listPasskeys();
      setPasskeys(body.passkeys);
      setReauthAfterMs(body.reauth_after_ms);
      setListState("ready");
    } catch (reason) {
      setListState("error");
      setLoadError(
        reason instanceof AuthApiError
          ? reason.message
          : "Could not load registered Passkeys.",
      );
    }
  }, []);

  useEffect(() => {
    if (me.auth_enabled === false || policy?.enabled === false) return;
    void load();
  }, [load, me.auth_enabled, policy?.enabled]);

  const publishMe = (next: ProductMe): void => {
    updateMe(next);
    onMe?.(next);
  };

  const add = (): void => {
    if (busy || !passkeyFlowSupported() || nickname.trim() === "") return;
    const requestedNickname = nickname.trim();
    setBusy(true);
    setError(null);
    setNotice(null);
    void (async () => {
      const created = await retryWithRecentProductAuth(
        () => registerPasskey(requestedNickname),
        reauthenticate,
      );
      setPasskeys((current) => [
        created,
        ...current.filter((passkey) => passkey.id !== created.id),
      ]);
      setListState("ready");
      setLoadError(null);
      setNickname("");
      setShowAddAnother(false);
      setNotice({
        severity: "success",
        message: `${created.nickname} was added and is ready to use.`,
      });
      publishMe({
        ...me,
        passkey_count: Math.max(1, (me.passkey_count ?? 0) + 1),
      });

      void authApi.listPasskeys().then((body) => {
        setPasskeys(body.passkeys);
        setReauthAfterMs(body.reauth_after_ms);
      }).catch(() => undefined);
      void authApi.me().then(publishMe).catch(() => undefined);
    })()
      .catch((reason: unknown) => {
        if (passkeyFlowCancelled(reason)) {
          setNotice({
            severity: "info",
            message: "Passkey setup was cancelled. Nothing changed.",
          });
          return;
        }
        setError(passkeyErrorMessage(reason, "Could not add a Passkey"));
      })
      .finally(() => setBusy(false));
  };

  const revoke = (id: string): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    void retryWithRecentProductAuth(
      () => authApi.deletePasskey(id),
      reauthenticate,
    )
      .then(() => {
        const remaining = passkeys.filter((passkey) => passkey.id !== id);
        setPasskeys(remaining);
        if (remaining.length === 0) setEnabled(false);
        publishMe({
          ...me,
          passkey_count: remaining.length,
          passkey_reauth_enabled: remaining.length === 0 ? false : enabled,
        });
        setNotice({
          severity: "success",
          message: "Passkey revoked from this Cowboy account.",
        });
        void authApi.me().then(publishMe).catch(() => undefined);
      })
      .catch((reason: unknown) => {
        if (passkeyFlowCancelled(reason)) {
          setNotice({ severity: "info", message: "Revocation was cancelled." });
          return;
        }
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not revoke Passkey",
        );
      })
      .finally(() => setBusy(false));
  };

  const toggle = (next: boolean): void => {
    setBusy(true);
    setError(null);
    setNotice(null);
    void (async () => {
      let updated = await authApi.setPasskeyReauth(next, reauthAfterMs);
      if (next) {
        try {
          updated = await verifyPasskey();
        } catch (reason) {
          await authApi.setPasskeyReauth(false, reauthAfterMs).catch(() =>
            undefined
          );
          throw reason;
        }
      }
      return updated;
    })()
      .then((updated) => {
        setEnabled(updated.passkey_reauth_enabled === true);
        publishMe(updated);
        setNotice({
          severity: "success",
          message: updated.passkey_reauth_enabled === true
            ? "Periodic Passkey verification is on."
            : "Periodic Passkey verification is off.",
        });
      })
      .catch((reason: unknown) => {
        if (passkeyFlowCancelled(reason)) {
          setEnabled(false);
          setNotice({
            severity: "info",
            message: "Verification was cancelled. Periodic checks remain off.",
          });
          return;
        }
        setError(passkeyErrorMessage(reason, "Could not save setting"));
      })
      .finally(() => setBusy(false));
  };

  const changeInterval = (next: number): void => {
    setBusy(true);
    setError(null);
    setNotice(null);
    void authApi
      .setPasskeyReauth(enabled, next)
      .then((updated) => {
        setReauthAfterMs(updated.passkey_reauth_after_ms ?? next);
        publishMe(updated);
        setNotice({
          severity: "success",
          message: "Verification frequency updated.",
        });
      })
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not save setting",
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
  const canCreate = passkeyFlowSupported();
  const addForm = (
    <Box sx={CARD_SX}>
      <Stack spacing={1.5}>
        <Box>
          <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
            {passkeys.length === 0
              ? "Add your first Passkey"
              : "Add another Passkey"}
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.4 }}>
            Give it a recognizable name, such as “iPhone” or “MacBook”. Cowboy
            never receives the private key.
          </Typography>
        </Box>
        <Stack
          direction={{ xs: "column", sm: "row" }}
          spacing={1}
          alignItems={{ sm: "center" }}
        >
          <TextField
            size="small"
            label="Passkey name"
            value={nickname}
            onChange={(event) => setNickname(event.target.value)}
            slotProps={{
              htmlInput: {
                maxLength: 64,
                autoComplete: "off",
                enterKeyHint: "done",
              },
            }}
            disabled={busy || !canCreate}
            fullWidth
          />
          <Button
            variant="contained"
            size="large"
            startIcon={busy
              ? <CircularProgress size={16} color="inherit" />
              : <AddRounded />}
            disabled={busy || !canCreate || nickname.trim() === ""}
            onClick={add}
            sx={{ minWidth: { sm: 112 } }}
          >
            Add
          </Button>
        </Stack>
        {!canCreate && (
          <Alert severity="info">This browser cannot create a Passkey.</Alert>
        )}
      </Stack>
    </Box>
  );

  return (
    <Stack spacing={2} data-product-passkeys-panel>
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={1.5}
      >
        <Stack direction="row" spacing={1.25} alignItems="center">
          <Box
            sx={{
              alignItems: "center",
              bgcolor: "action.hover",
              borderRadius: 2,
              display: "flex",
              height: 40,
              justifyContent: "center",
              width: 40,
            }}
          >
            <KeyRounded color="primary" />
          </Box>
          <Box>
            <Typography variant="subtitle1" sx={{ fontWeight: 750 }}>
              Passkeys
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Phishing-resistant protection for this account
            </Typography>
          </Box>
        </Stack>
        <Chip
          size="small"
          variant="outlined"
          color={listState === "ready" && passkeys.length > 0
            ? "success"
            : "default"}
          label={listState === "loading"
            ? "Checking…"
            : listState === "error"
            ? "Unavailable"
            : passkeys.length === 0
            ? "Not set up"
            : `${passkeys.length} registered`}
        />
      </Stack>

      <Typography variant="body2" color="text.secondary">
        Sign-in is still required. A Passkey adds secure verification and can
        extend this device&apos;s session after you explicitly unlock it.
      </Typography>

      {notice && <Alert severity={notice.severity}>{notice.message}</Alert>}
      {error && <Alert severity="error">{error}</Alert>}

      {listState === "loading" && (
        <Box sx={CARD_SX}>
          <Stack direction="row" spacing={1.25} alignItems="center">
            <CircularProgress size={20} />
            <Typography variant="body2" color="text.secondary">
              Checking registered Passkeys…
            </Typography>
          </Stack>
        </Box>
      )}

      {listState === "error" && (
        <Alert
          severity="warning"
          action={
            <Button color="inherit" size="small" onClick={() => void load()}>
              Retry
            </Button>
          }
        >
          {loadError}
        </Alert>
      )}

      {listState === "ready" && passkeys.length === 0 && addForm}

      {listState === "ready" && passkeys.length > 0 && (
        <>
          <Box sx={CARD_SX}>
            <Stack spacing={1.5}>
              <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
                Registered Passkeys
              </Typography>
              {passkeys.map((passkey) => (
                <Stack
                  key={passkey.id}
                  direction="row"
                  spacing={1.25}
                  alignItems="center"
                  justifyContent="space-between"
                  sx={{
                    borderTop: 1,
                    borderColor: "divider",
                    pt: 1.5,
                  }}
                >
                  <Stack
                    direction="row"
                    spacing={1.1}
                    alignItems="center"
                    minWidth={0}
                  >
                    <KeyRounded fontSize="small" color="action" />
                    <Box minWidth={0}>
                      <Typography variant="body2" sx={{ fontWeight: 700 }}>
                        {passkey.nickname}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        Added {passkeyDate(passkey.created_at_ms)}
                      </Typography>
                    </Box>
                  </Stack>
                  <Button
                    color="error"
                    variant="outlined"
                    size="small"
                    disabled={busy}
                    onClick={() => revoke(passkey.id)}
                  >
                    Revoke
                  </Button>
                </Stack>
              ))}
            </Stack>
          </Box>

          {refreshEnabled
            ? (
              <Box sx={CARD_SX}>
                <Stack spacing={1.5}>
                  <FormControlLabel
                    sx={{
                      alignItems: "flex-start",
                      justifyContent: "space-between",
                      m: 0,
                    }}
                    labelPlacement="start"
                    control={
                      <Switch
                        checked={enabled}
                        disabled={busy}
                        onChange={(event) => toggle(event.target.checked)}
                        slotProps={{
                          input: {
                            "aria-label": "Require Passkey periodically",
                          },
                        }}
                      />
                    }
                    label={
                      <Box sx={{ pr: 1.5 }}>
                        <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
                          Periodic verification
                        </Typography>
                        <Typography
                          variant="body2"
                          color="text.secondary"
                          sx={{ mt: 0.35 }}
                        >
                          Lock only this view when verification is due. Running
                          agents continue in the background.
                        </Typography>
                      </Box>
                    }
                  />
                  <FormControl size="small" disabled={busy} fullWidth>
                    <InputLabel id="passkey-refresh-interval-label">
                      Verification frequency
                    </InputLabel>
                    <Select
                      labelId="passkey-refresh-interval-label"
                      label="Verification frequency"
                      value={reauthAfterMs}
                      onChange={(event) =>
                        changeInterval(Number(event.target.value))}
                    >
                      {REFRESH_INTERVALS.map((option) => (
                        <MenuItem key={option.value} value={option.value}>
                          {option.label}
                        </MenuItem>
                      ))}
                    </Select>
                  </FormControl>
                  <Typography variant="caption" color="text.secondary">
                    Periodic verification is off until you enable it. Enabling
                    it asks for the Passkey once immediately.
                  </Typography>
                </Stack>
              </Box>
            )
            : (
              <Alert severity="info">
                Periodic Passkey verification is disabled by this Cowboy
                Service.
              </Alert>
            )}

          {showAddAnother
            ? addForm
            : (
              <Button
                variant="outlined"
                startIcon={<AddRounded />}
                onClick={() => setShowAddAnother(true)}
                sx={{ alignSelf: "flex-start" }}
              >
                Add another Passkey
              </Button>
            )}
        </>
      )}
    </Stack>
  );
}
