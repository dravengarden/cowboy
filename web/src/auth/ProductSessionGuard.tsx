import {
  AccessTimeRounded,
  CloseRounded,
  FingerprintRounded,
  LoginRounded,
} from "@mui/icons-material";
import {
  Box,
  Button,
  IconButton,
  Paper,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import type {
  ProductMe,
  ProductOidcProvider,
  ProductSessionServerPolicy,
} from "./authApi";
import { ProductRecentAuthSheet } from "./ProductRecentAuthSheet";
import {
  nextSessionClockDelay,
  sessionAlertState,
  sessionCountdownLabel,
} from "./sessionSchedule";

const FINAL_WARNING_MS = 30 * 60 * 1_000;

export function ProductSessionGuard({
  me,
  policy,
  providers,
  passwordEnabled,
  loginMethodOrder,
  suspended,
  onVerified,
  onSignOut,
}: {
  me: ProductMe;
  policy: ProductSessionServerPolicy | undefined;
  providers: ProductOidcProvider[];
  passwordEnabled: boolean;
  loginMethodOrder: string[];
  suspended: boolean;
  onVerified: (me: ProductMe) => void;
  onSignOut: () => Promise<void>;
}): React.JSX.Element | null {
  const surface = useSurfaceProfile();
  const mobile = surface.kind !== "desktop";
  const [clientNow, setClientNow] = useState(Date.now);
  const [clockOffset, setClockOffset] = useState(() =>
    typeof me.session_server_now_ms === "number"
      ? me.session_server_now_ms - Date.now()
      : 0
  );
  const serverNow = clientNow + clockOffset;
  const alert = useMemo(
    () => sessionAlertState(me, serverNow),
    [me, serverNow],
  );
  const alertKey = alert ? `${alert.kind}:${alert.dueAtMs}` : "";
  const [dialogOpen, setDialogOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    if (typeof me.session_server_now_ms === "number") {
      setClockOffset(me.session_server_now_ms - Date.now());
      setClientNow(Date.now());
    }
  }, [me.session_server_now_ms]);

  useEffect(() => {
    const timer = globalThis.setTimeout(
      () => setClientNow(Date.now()),
      nextSessionClockDelay(me, serverNow, alert),
    );
    return () => globalThis.clearTimeout(timer);
  }, [alert, me, serverNow]);

  useEffect(() => {
    setDialogOpen(false);
    if (!alertKey) {
      setCollapsed(false);
      return;
    }
    try {
      setCollapsed(
        globalThis.sessionStorage.getItem(
          `cowboy-session-warning:${alertKey}`,
        ) === "collapsed",
      );
    } catch {
      setCollapsed(false);
    }
  }, [alertKey]);

  if (me.auth_enabled === false || !policy || !alert) return null;

  const remaining = Math.max(0, alert.dueAtMs - serverNow);
  const required = alert.phase === "required";
  const mandatory = required || remaining <= FINAL_WARNING_MS;
  const sheetOpen = !suspended && (dialogOpen || required);
  const label = sessionCountdownLabel(remaining);
  const title = required
    ? alert.kind === "passkey" ? "Passkey check required" : "Sign-in required"
    : alert.kind === "passkey"
    ? `Passkey check in ${label}`
    : `Sign in again within ${label}`;
  const detail = alert.kind === "passkey"
    ? "Verify without interrupting running agents."
    : "Use your password or an enabled login provider. Running agents keep working.";
  const lockBackdrop = required && sheetOpen && globalThis.document?.body
    ? createPortal(
      <Box
        aria-hidden
        data-session-lock-backdrop="true"
        sx={{
          position: "fixed",
          inset: 0,
          zIndex: 1249,
          pointerEvents: "auto",
          bgcolor: (theme) =>
            theme.palette.mode === "dark"
              ? "rgba(7, 5, 13, 0.88)"
              : "rgba(247, 246, 250, 0.9)",
          backdropFilter: "blur(24px) saturate(65%)",
          WebkitBackdropFilter: "blur(24px) saturate(65%)",
        }}
      />,
      globalThis.document.body,
    )
    : null;

  const collapse = (): void => {
    if (mandatory) return;
    try {
      globalThis.sessionStorage.setItem(
        `cowboy-session-warning:${alertKey}`,
        "collapsed",
      );
    } catch {
      // The compact state still lasts for this mount when storage is unavailable.
    }
    setCollapsed(true);
  };

  return (
    <>
      {!sheetOpen && (
        <Box
          sx={{
            position: "fixed",
            top: mobile
              ? "calc(max(env(safe-area-inset-top, 0px), var(--cowboy-system-top-clearance, 0px)) + 8px)"
              : 12,
            left: "50%",
            transform: "translateX(-50%)",
            width: mobile
              ? "calc(100% - 24px)"
              : "min(760px, calc(100% - 48px))",
            zIndex: (theme) => theme.zIndex.tooltip + 2,
            pointerEvents: "none",
          }}
        >
          {collapsed && !mandatory
            ? (
              <Paper
                role="status"
                elevation={10}
                onClick={() => setDialogOpen(true)}
                sx={{
                  pointerEvents: "auto",
                  ml: "auto",
                  width: "fit-content",
                  border: 1,
                  borderColor: "warning.main",
                  borderRadius: 999,
                  px: 1.25,
                  py: 0.75,
                  cursor: "pointer",
                }}
              >
                <Stack direction="row" spacing={0.75} alignItems="center">
                  <AccessTimeRounded fontSize="small" color="warning" />
                  <Typography variant="body2" sx={{ fontWeight: 700 }}>
                    {label}
                  </Typography>
                </Stack>
              </Paper>
            )
            : (
              <Paper
                role="status"
                aria-live={mandatory ? "assertive" : "polite"}
                elevation={12}
                sx={{
                  pointerEvents: "auto",
                  border: 1,
                  borderColor: mandatory ? "error.main" : "warning.main",
                  borderRadius: mobile ? 2.5 : 999,
                  px: mobile ? 1.5 : 2,
                  py: mobile ? 1.25 : 0.75,
                  bgcolor: "background.paper",
                }}
              >
                <Stack
                  direction={mobile ? "column" : "row"}
                  spacing={mobile ? 1 : 1.5}
                  alignItems={mobile ? "stretch" : "center"}
                >
                  <Stack
                    direction="row"
                    spacing={1}
                    alignItems="center"
                    sx={{ minWidth: 0, flex: 1 }}
                  >
                    {alert.kind === "passkey"
                      ? (
                        <FingerprintRounded
                          color={mandatory ? "error" : "warning"}
                        />
                      )
                      : (
                        <LoginRounded color={mandatory ? "error" : "warning"} />
                      )}
                    <Box sx={{ minWidth: 0, flex: 1 }}>
                      <Typography variant="body2" sx={{ fontWeight: 800 }}>
                        {title}
                      </Typography>
                      {!mobile && (
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          noWrap
                        >
                          {detail}
                        </Typography>
                      )}
                    </Box>
                    {!mandatory && mobile && (
                      <Tooltip title="Keep a compact reminder">
                        <IconButton
                          size="small"
                          onClick={collapse}
                          aria-label="Collapse session reminder"
                        >
                          <CloseRounded fontSize="small" />
                        </IconButton>
                      </Tooltip>
                    )}
                  </Stack>
                  <Stack
                    direction="row"
                    spacing={1}
                    alignItems="center"
                    justifyContent="flex-end"
                  >
                    <Button
                      variant={mandatory ? "contained" : "outlined"}
                      color={mandatory ? "error" : "warning"}
                      size="small"
                      onClick={() => setDialogOpen(true)}
                      sx={{ minHeight: mobile ? 44 : 34, whiteSpace: "nowrap" }}
                    >
                      {alert.kind === "passkey" ? "Verify now" : "Sign in now"}
                    </Button>
                    {!mandatory && !mobile && (
                      <Tooltip title="Keep a compact reminder">
                        <IconButton
                          size="small"
                          onClick={collapse}
                          aria-label="Collapse session reminder"
                        >
                          <CloseRounded fontSize="small" />
                        </IconButton>
                      </Tooltip>
                    )}
                  </Stack>
                </Stack>
              </Paper>
            )}
        </Box>
      )}
      {lockBackdrop}
      <ProductRecentAuthSheet
        open={sheetOpen}
        me={me}
        providers={providers}
        passwordEnabled={passwordEnabled}
        loginMethodOrder={loginMethodOrder}
        purpose={alert.kind}
        locked={required}
        onVerified={(next) => {
          setDialogOpen(false);
          onVerified(next);
        }}
        onCancel={() => setDialogOpen(false)}
        onSignOut={() => void onSignOut()}
      />
    </>
  );
}
