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
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { authApi } from "./authApi";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";
import { sessionCountdownLabel } from "./sessionSchedule";
import {
  captureDeviceAuthorizationFromLocation,
  clearDeviceAuthorization,
  DeviceAuthorizationFlow,
  sameDeviceAuthorization,
  storedDeviceAuthorization,
} from "./deviceAuthorization";

export { captureDeviceAuthorizationFromLocation } from "./deviceAuthorization";

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
  const [request, setRequest] = useState(() => storedDeviceAuthorization());
  const { reauthenticate } = useProductAuth();
  const flow = useMemo(() =>
    new DeviceAuthorizationFlow(request, {
      inspect: authApi.inspectDeviceAuthorization,
      approve: authApi.approveDeviceAuthorization,
      deny: authApi.denyDeviceAuthorization,
      authorize: (operation) =>
        retryWithRecentProductAuth(operation, reauthenticate),
      clear: clearDeviceAuthorization,
    }), [request, reauthenticate]);
  const { phase, info, remainingMs, busy, error } = useSyncExternalStore(
    flow.subscribe,
    flow.getSnapshot,
    flow.getSnapshot,
  );
  const needsNewLink = ["expired", "unavailable", "missing"].includes(phase);

  useEffect(() => {
    const capture = (): void => {
      if (globalThis.location.pathname !== "/auth/device") return;
      captureDeviceAuthorizationFromLocation();
      const next = storedDeviceAuthorization();
      setRequest((previous) =>
        sameDeviceAuthorization(previous, next) ? previous : next
      );
    };
    // Opening a fresh link in this tab may only change the fragment: neither
    // React nor the document is remounted by that navigation.
    globalThis.addEventListener("hashchange", capture);
    globalThis.addEventListener("popstate", capture);
    capture();
    return () => {
      globalThis.removeEventListener("hashchange", capture);
      globalThis.removeEventListener("popstate", capture);
    };
  }, []);

  useEffect(() => {
    void flow.inspect();
    return flow.stop;
  }, [flow]);

  useEffect(() => {
    if (phase !== "pending") return;
    const timer = globalThis.setInterval(flow.tick, 1_000);
    const onVisible = (): void => {
      if (document.visibilityState === "visible") flow.tick();
    };
    globalThis.addEventListener("focus", flow.tick);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      globalThis.clearInterval(timer);
      globalThis.removeEventListener("focus", flow.tick);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [flow, phase]);

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
            {error && (
              <>
                <Alert severity="warning">{error}</Alert>
                <Button
                  variant="outlined"
                  size="large"
                  disabled={busy}
                  onClick={() => void flow.inspect()}
                >
                  Check request again
                </Button>
              </>
            )}
            {needsNewLink && (
              <>
                <Alert severity="warning">
                  {phase === "expired"
                    ? "This authorization request has expired."
                    : phase === "missing"
                    ? "No valid authorization link was found."
                    : "This authorization link has expired or is no longer available."}
                </Alert>
                <Typography sx={{ fontWeight: 700 }}>
                  Start again from the requesting app
                </Typography>
                <Typography color="text.secondary">
                  Return to the app or terminal that requested access. If it has
                  not connected, restart sign-in there and open the new link
                  within 5 minutes. Refreshing this page cannot renew the old
                  request.
                </Typography>
                <Button component="a" href="/" variant="outlined" size="large">
                  Back to Cowboy
                </Button>
              </>
            )}
            {phase === "loading" && !error && (
              <Box
                sx={{ minHeight: 180, display: "grid", placeItems: "center" }}
              >
                <CircularProgress size={26} color="inherit" />
              </Box>
            )}
            {info && phase === "pending" && (
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
                      <Typography sx={{ fontWeight: 700 }}>
                        {info.name}
                      </Typography>
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
                  <Typography
                    component="span"
                    role="timer"
                    aria-live="off"
                    sx={{ fontWeight: 700 }}
                  >
                    Expires in {sessionCountdownLabel(remainingMs)}.
                  </Typography>{" "}
                  Keep the requesting app or terminal open until it confirms the
                  connection. You can revoke the device later in Settings →
                  Account.
                </Alert>
                <Stack
                  direction={{ xs: "column-reverse", sm: "row" }}
                  spacing={1}
                >
                  <Button
                    variant="outlined"
                    color="inherit"
                    size="large"
                    fullWidth
                    disabled={busy}
                    onClick={() => void flow.deny()}
                  >
                    Deny
                  </Button>
                  <Button
                    variant="contained"
                    size="large"
                    fullWidth
                    disabled={busy}
                    onClick={() => void flow.approve()}
                  >
                    {busy ? "Authorizing…" : "Authorize client"}
                  </Button>
                </Stack>
              </>
            )}
            {phase === "approved" && (
              <Stack
                spacing={1.5}
                alignItems="center"
                sx={{ py: 3, textAlign: "center" }}
              >
                <CheckCircleOutline color="success" sx={{ fontSize: 52 }} />
                <Typography variant="h6" sx={{ fontWeight: 750 }}>
                  Client authorized
                </Typography>
                <Typography color="text.secondary">
                  Return to the app or terminal. This window can now be closed.
                </Typography>
              </Stack>
            )}
            {phase === "denied" && (
              <Stack
                spacing={1.5}
                alignItems="center"
                sx={{ py: 3, textAlign: "center" }}
              >
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
