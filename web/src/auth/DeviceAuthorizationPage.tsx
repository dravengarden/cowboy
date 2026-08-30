import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Divider,
  Stack,
  Typography,
} from "@mui/material";
import {
  CheckCircleOutline,
  ComputerOutlined,
  Fingerprint,
  ShieldOutlined,
} from "@mui/icons-material";
import { useEffect, useMemo, useState } from "react";
import {
  AuthApiError,
  authApi,
  type DeviceAuthorizationInfo,
  type DeviceAuthorizationRequest,
} from "./authApi";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";

const DEVICE_AUTH_STORAGE_KEY = "cowboy:pending-device-authorization";

function validCapability(value: string | null): value is string {
  return value !== null && /^[A-Za-z0-9_-]{20,128}$/u.test(value);
}

export function captureDeviceAuthorizationFromLocation(): boolean {
  if (globalThis.location.pathname === "/auth/device") {
    const values = new URLSearchParams(globalThis.location.hash.slice(1));
    const requestId = values.get("request_id");
    const approvalToken = values.get("approval_token");
    if (validCapability(requestId) && validCapability(approvalToken)) {
      globalThis.sessionStorage.setItem(
        DEVICE_AUTH_STORAGE_KEY,
        JSON.stringify({
          request_id: requestId,
          approval_token: approvalToken,
        } satisfies DeviceAuthorizationRequest),
      );
      globalThis.history.replaceState(null, "", "/auth/device");
    }
  }
  return globalThis.sessionStorage.getItem(DEVICE_AUTH_STORAGE_KEY) !== null ||
    globalThis.location.pathname === "/auth/device";
}

function storedDeviceAuthorization(): DeviceAuthorizationRequest | null {
  const stored = globalThis.sessionStorage.getItem(DEVICE_AUTH_STORAGE_KEY);
  if (!stored) return null;
  try {
    const value = JSON.parse(stored) as Partial<DeviceAuthorizationRequest>;
    const requestId = value.request_id ?? null;
    const approvalToken = value.approval_token ?? null;
    return validCapability(requestId) && validCapability(approvalToken)
      ? { request_id: requestId, approval_token: approvalToken }
      : null;
  } catch {
    return null;
  }
}

function clearDeviceAuthorization(): void {
  globalThis.sessionStorage.removeItem(DEVICE_AUTH_STORAGE_KEY);
}

export function DeviceAuthorizationRoute({
  active,
  children,
}: {
  active: boolean;
  children: React.ReactNode;
}): React.JSX.Element {
  return active ? <DeviceAuthorizationPage /> : <>{children}</>;
}

export function DeviceAuthorizationPage(): React.JSX.Element {
  const request = useMemo(storedDeviceAuthorization, []);
  const { reauthenticate } = useProductAuth();
  const [info, setInfo] = useState<DeviceAuthorizationInfo | null>(null);
  const [result, setResult] = useState<"approved" | "denied" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(
    request ? null : "This authorization link is missing or no longer available.",
  );

  useEffect(() => {
    if (!request) return;
    void authApi.inspectDeviceAuthorization(request)
      .then((authorization) => {
        setInfo(authorization);
        if (authorization.status === "approved") setResult("approved");
        if (authorization.status === "denied") setResult("denied");
      })
      .catch((reason: unknown) => {
        clearDeviceAuthorization();
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not inspect this authorization request.",
        );
      });
  }, [request]);

  const approve = (): void => {
    if (!request || busy) return;
    setBusy(true);
    setError(null);
    void retryWithRecentProductAuth(
      () => authApi.approveDeviceAuthorization(request),
      reauthenticate,
    )
      .then(() => {
        clearDeviceAuthorization();
        setResult("approved");
      })
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not approve this device.",
        );
      })
      .finally(() => setBusy(false));
  };

  const deny = (): void => {
    if (!request || busy) return;
    setBusy(true);
    setError(null);
    void authApi.denyDeviceAuthorization(request)
      .then(() => {
        clearDeviceAuthorization();
        setResult("denied");
      })
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not deny this request.",
        );
      })
      .finally(() => setBusy(false));
  };

  return (
    <Box
      sx={{
        minHeight: "100%",
        display: "grid",
        placeItems: "center",
        bgcolor: "background.default",
        px: 2,
        py: "max(24px, env(safe-area-inset-top))",
      }}
    >
      <Stack spacing={2.5} sx={{ width: "100%", maxWidth: 520 }}>
        <Stack direction="row" spacing={1.5} alignItems="center">
          <Box
            sx={{
              width: 48,
              height: 48,
              borderRadius: "50%",
              display: "grid",
              placeItems: "center",
              bgcolor: "action.hover",
              color: "primary.main",
            }}
          >
            <ShieldOutlined />
          </Box>
          <Box>
            <Typography sx={{ letterSpacing: "0.14em", fontSize: 12 }}>
              COWBOY SECURITY
            </Typography>
            <Typography variant="h5" sx={{ fontWeight: 750 }}>
              Authorize this client
            </Typography>
          </Box>
        </Stack>

        <Box
          sx={{
            border: 1,
            borderColor: "divider",
            borderRadius: 4,
            bgcolor: "background.paper",
            overflow: "hidden",
          }}
        >
          <Stack spacing={2.25} sx={{ p: { xs: 2.25, sm: 3 } }}>
            {error && <Alert severity="error">{error}</Alert>}
            {!info && !error && (
              <Box sx={{ minHeight: 180, display: "grid", placeItems: "center" }}>
                <CircularProgress size={26} color="inherit" />
              </Box>
            )}
            {info && result === null && (
              <>
                <Typography color="text.secondary">
                  Approve only if you just started Cowboy on this computer or
                  app. Approval grants access as your signed-in account; it
                  never shares your password or Passkey.
                </Typography>
                <Divider />
                <Stack spacing={2}>
                  <Stack direction="row" spacing={1.5} alignItems="center">
                    <ComputerOutlined color="action" />
                    <Box sx={{ minWidth: 0 }}>
                      <Typography variant="caption" color="text.secondary">
                        Client
                      </Typography>
                      <Typography sx={{ fontWeight: 700 }}>{info.name}</Typography>
                    </Box>
                  </Stack>
                  <Stack direction="row" spacing={1.5} alignItems="center">
                    <Fingerprint color="action" />
                    <Box sx={{ minWidth: 0 }}>
                      <Typography variant="caption" color="text.secondary">
                        Public-key fingerprint
                      </Typography>
                      <Typography
                        sx={{
                          fontFamily: "ui-monospace, SFMono-Regular, monospace",
                          overflowWrap: "anywhere",
                        }}
                      >
                        {info.fingerprint}
                      </Typography>
                    </Box>
                  </Stack>
                </Stack>
                <Alert severity="info">
                  This one-time request expires automatically. You can revoke
                  the device later in Settings → Account.
                </Alert>
                <Stack direction={{ xs: "column-reverse", sm: "row" }} spacing={1}>
                  <Button
                    variant="outlined"
                    color="inherit"
                    size="large"
                    fullWidth
                    disabled={busy}
                    onClick={deny}
                  >
                    Deny
                  </Button>
                  <Button
                    variant="contained"
                    size="large"
                    fullWidth
                    disabled={busy}
                    onClick={approve}
                  >
                    {busy ? "Authorizing…" : "Authorize client"}
                  </Button>
                </Stack>
              </>
            )}
            {result === "approved" && (
              <Stack spacing={1.5} alignItems="center" sx={{ py: 3, textAlign: "center" }}>
                <CheckCircleOutline color="success" sx={{ fontSize: 52 }} />
                <Typography variant="h6" sx={{ fontWeight: 750 }}>
                  Client authorized
                </Typography>
                <Typography color="text.secondary">
                  Return to the app or terminal. This window can now be closed.
                </Typography>
              </Stack>
            )}
            {result === "denied" && (
              <Stack spacing={1.5} alignItems="center" sx={{ py: 3, textAlign: "center" }}>
                <ShieldOutlined color="action" sx={{ fontSize: 48 }} />
                <Typography variant="h6" sx={{ fontWeight: 750 }}>
                  Request denied
                </Typography>
                <Typography color="text.secondary">
                  No device credential was created. You can close this window.
                </Typography>
              </Stack>
            )}
          </Stack>
        </Box>
      </Stack>
    </Box>
  );
}
