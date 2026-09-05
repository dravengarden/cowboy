import AddRounded from "@mui/icons-material/AddRounded";
import KeyRounded from "@mui/icons-material/KeyRounded";
import SecurityRounded from "@mui/icons-material/SecurityRounded";
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
  type ProductPasskeyServerPolicy,
  type ProductSessionServerPolicy,
} from "./authApi";
import {
  passkeyErrorMessage,
  passkeyFlowCancelled,
  passkeyFlowSupported,
  passkeyRegistrationNeedsUserGestureResume,
  registerPasskey,
  verifyPasskey,
} from "./passkeyFlow";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";
import {
  DEFAULT_PASSKEY_REAUTH_INTERVAL_MS,
  normalizePasskeyReauthInterval,
  PASSKEY_REAUTH_INTERVALS,
} from "./passkeyIntervals";
import {
  configuredSessionProtectionItems,
  currentSessionProtectionItems,
  sessionDeadlineLabel,
  sessionPolicyDuration,
} from "./sessionProtection";
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

function SessionPolicySummary({
  me,
  passkeys,
  policy,
}: {
  me: ProductMe;
  passkeys: ProductPasskeyServerPolicy | undefined;
  policy: ProductSessionServerPolicy | undefined;
}): React.JSX.Element | null {
  const [serverNowMs, setServerNowMs] = useState(
    me.session_server_now_ms ?? Date.now(),
  );
  useEffect(() => {
    const offset = (me.session_server_now_ms ?? Date.now()) - Date.now();
    let timer = 0;
    const tick = (): void => {
      setServerNowMs(Date.now() + offset);
      timer = globalThis.setTimeout(tick, 60_000);
    };
    tick();
    return () => globalThis.clearTimeout(timer);
  }, [me.session_server_now_ms]);
  if (!policy || !passkeys) return null;
  const currentRows = currentSessionProtectionItems(
    me,
    passkeys,
    policy,
    serverNowMs,
  );
  const configuredRows = configuredSessionProtectionItems(passkeys, policy);
  return (
    <Box sx={CARD_SX} data-product-session-policy>
      <Stack spacing={2}>
        <Stack
          direction="row"
          spacing={1.25}
          alignItems="center"
          justifyContent="space-between"
        >
          <Stack direction="row" spacing={1.1} alignItems="center" minWidth={0}>
            <Box
              sx={{
                alignItems: "center",
                bgcolor: "action.hover",
                borderRadius: 2,
                color: "primary.main",
                display: "flex",
                flex: "0 0 auto",
                height: 40,
                justifyContent: "center",
                width: 40,
              }}
            >
              <SecurityRounded />
            </Box>
            <Box minWidth={0}>
              <Typography variant="subtitle1" sx={{ fontWeight: 750 }}>
                Session protection
              </Typography>
              <Typography variant="body2" color="text.secondary">
                This browser and the Cowboy Service policy
              </Typography>
            </Box>
          </Stack>
          <Chip
            size="small"
            color="success"
            variant="outlined"
            label="Signed in"
          />
        </Stack>

        <Box
          data-current-session-protection
          sx={{
            display: "grid",
            gap: 1,
            gridTemplateColumns: {
              xs: "repeat(2, minmax(0, 1fr))",
              md: "repeat(4, minmax(0, 1fr))",
            },
          }}
        >
          {currentRows.map((row) => (
            <Box
              key={row.label}
              sx={{
                bgcolor: "action.hover",
                borderRadius: 2,
                minWidth: 0,
                px: 1.25,
                py: 1.1,
              }}
            >
              <Typography variant="caption" color="text.secondary">
                {row.label}
              </Typography>
              <Typography variant="body2" sx={{ fontWeight: 750, mt: 0.2 }}>
                {row.value}
              </Typography>
            </Box>
          ))}
        </Box>

        <Box>
          <Stack
            direction="row"
            alignItems="baseline"
            justifyContent="space-between"
            spacing={2}
            sx={{ mb: 1 }}
          >
            <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
              Service settings
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Read-only
            </Typography>
          </Stack>
          <Box
            data-session-service-settings
            sx={{
              display: "grid",
              gap: 1,
              gridTemplateColumns: {
                xs: "repeat(2, minmax(0, 1fr))",
                md: "repeat(3, minmax(0, 1fr))",
              },
            }}
          >
            {configuredRows.map((row) => (
              <Box
                key={row.label}
                sx={{
                  bgcolor: "action.hover",
                  borderRadius: 2,
                  minWidth: 0,
                  px: 1.25,
                  py: 1.05,
                }}
              >
                <Typography variant="caption" color="text.secondary">
                  {row.label}
                </Typography>
                <Typography variant="body2" sx={{ fontWeight: 700, mt: 0.2 }}>
                  {row.value}
                </Typography>
              </Box>
            ))}
          </Box>
        </Box>

        <Typography variant="caption" color="text.secondary">
          Browser sessions and Passkeys are shown here. Authorized Cowboy CLI
          and ACP clients are listed separately; activity never extends the
          Passkey or full sign-in hard deadlines.
        </Typography>
      </Stack>
    </Box>
  );
}

export function ProductPasskeysPanel({
  onMe,
}: {
  onMe?: (me: ProductMe) => void;
}): React.JSX.Element {
  const {
    me,
    passkeys: policy,
    session: sessionPolicy,
    reauthenticate,
    updateMe,
  } = useProductAuth();
  const [passkeys, setPasskeys] = useState<ProductPasskey[]>([]);
  const [listState, setListState] = useState<ListState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [nickname, setNickname] = useState("");
  const [showAddAnother, setShowAddAnother] = useState(false);
  const [enabled, setEnabled] = useState(me.passkey_reauth_enabled === true);
  const [reauthAfterMs, setReauthAfterMs] = useState(
    normalizePasskeyReauthInterval(
      me.passkey_reauth_after_ms ?? DEFAULT_PASSKEY_REAUTH_INTERVAL_MS,
      sessionPolicy?.passkey_max_age_ms ?? Number.MAX_SAFE_INTEGER,
    ),
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
      setReauthAfterMs(normalizePasskeyReauthInterval(
        body.reauth_after_ms,
        sessionPolicy?.passkey_max_age_ms ?? body.reauth_after_ms,
      ));
      setListState("ready");
    } catch (reason) {
      setListState("error");
      setLoadError(
        reason instanceof AuthApiError
          ? reason.message
          : "Could not load registered Passkeys.",
      );
    }
  }, [sessionPolicy?.passkey_max_age_ms]);

  useEffect(() => {
    if (me.auth_enabled === false || policy?.enabled === false) return;
    void load();
  }, [load, me.auth_enabled, policy?.enabled]);

  useEffect(() => {
    setEnabled(me.passkey_reauth_enabled === true);
  }, [me.passkey_reauth_enabled]);

  useEffect(() => {
    if (typeof me.passkey_reauth_after_ms !== "number") return;
    setReauthAfterMs(normalizePasskeyReauthInterval(
      me.passkey_reauth_after_ms,
      sessionPolicy?.passkey_max_age_ms ?? Number.MAX_SAFE_INTEGER,
    ));
  }, [me.passkey_reauth_after_ms, sessionPolicy?.passkey_max_age_ms]);

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
        {
          resumeLabel: "Continue to Passkey",
          resumeWithUserGesture: passkeyRegistrationNeedsUserGestureResume(),
        },
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
        setReauthAfterMs(normalizePasskeyReauthInterval(
          body.reauth_after_ms,
          sessionPolicy?.passkey_max_age_ms ?? body.reauth_after_ms,
        ));
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
        setReauthAfterMs(normalizePasskeyReauthInterval(
          updated.passkey_reauth_after_ms ?? next,
          sessionPolicy?.passkey_max_age_ms ?? Number.MAX_SAFE_INTEGER,
        ));
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

  const verifyCurrentSession = (): void => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    void verifyPasskey()
      .then((updated) => {
        publishMe(updated);
        setNotice({
          severity: "success",
          message: "This browser is now protected by periodic verification.",
        });
      })
      .catch((reason: unknown) => {
        if (passkeyFlowCancelled(reason)) {
          setNotice({
            severity: "info",
            message: "Verification was cancelled. Nothing changed.",
          });
          return;
        }
        setError(passkeyErrorMessage(reason, "Could not verify this browser"));
      })
      .finally(() => setBusy(false));
  };

  if (me.auth_enabled === false) return <></>;
  if (policy?.enabled === false) {
    return (
      <Stack spacing={2}>
        <SessionPolicySummary
          me={me}
          passkeys={policy}
          policy={sessionPolicy}
        />
        <Alert severity="info">
          Passkeys are disabled by this Cowboy Service.
        </Alert>
      </Stack>
    );
  }

  const refreshEnabled = policy?.session_refresh_enabled !== false;
  const refreshIntervals = PASSKEY_REAUTH_INTERVALS.filter((option) =>
    option.value <=
      (sessionPolicy?.passkey_max_age_ms ?? Number.MAX_SAFE_INTEGER)
  );
  const canCreate = passkeyFlowSupported();
  const knownPasskeyCount = listState === "ready"
    ? passkeys.length
    : me.passkey_count ?? 0;
  const passkeyStatusLabel = listState === "loading"
    ? "Checking…"
    : listState === "error"
    ? "Unavailable"
    : knownPasskeyCount === 0
    ? "Not set up"
    : enabled
    ? `${knownPasskeyCount} · checks on`
    : `${knownPasskeyCount} registered`;
  const passkeyDescription = knownPasskeyCount === 0
    ? "Optional phishing-resistant verification for this account."
    : enabled
    ? `Periodic verification is on for this account every ${
      sessionPolicyDuration(reauthAfterMs)
    }.`
    : "Ready to verify this account; periodic checks are off.";
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
      <SessionPolicySummary
        me={me}
        passkeys={policy}
        policy={sessionPolicy}
      />
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
          label={passkeyStatusLabel}
        />
      </Stack>

      <Typography variant="body2" color="text.secondary">
        {passkeyDescription}{" "}
        Sign-in is still required, and Cowboy never receives a Passkey private
        key.
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
            <Button
              color="inherit"
              size="small"
              onClick={() => void load()}
            >
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
                        <Typography
                          variant="subtitle2"
                          sx={{ fontWeight: 750 }}
                        >
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
                      {refreshIntervals.map((option) => (
                        <MenuItem key={option.value} value={option.value}>
                          {option.label}
                        </MenuItem>
                      ))}
                    </Select>
                  </FormControl>
                  <Typography variant="caption" color="text.secondary">
                    {enabled
                      ? me.passkey_reauth_due_at_ms == null
                        ? "Verify this browser once to start its periodic schedule. Other signed-in browsers keep their own verification state."
                        : `${
                          sessionDeadlineLabel(
                            me.passkey_reauth_due_at_ms,
                            me.session_server_now_ms ?? Date.now(),
                          )
                        }. The service allows at most ${
                          sessionPolicyDuration(
                            sessionPolicy?.passkey_max_age_ms ?? reauthAfterMs,
                          )
                        } between checks.`
                      : "Off for this account. Turning it on verifies the Passkey immediately; running agents remain active."}
                  </Typography>
                  {enabled && me.passkey_reauth_due_at_ms == null && (
                    <Button
                      variant="contained"
                      disabled={busy}
                      onClick={verifyCurrentSession}
                      startIcon={busy
                        ? <CircularProgress size={16} color="inherit" />
                        : <KeyRounded />}
                      sx={{ alignSelf: "flex-start" }}
                    >
                      Verify this browser
                    </Button>
                  )}
                </Stack>
              </Box>
            )
            : (
              <Alert severity="info">
                Periodic Passkey verification is disabled by this Cowboy
                Service.
              </Alert>
            )}

          {showAddAnother ? addForm : (
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
