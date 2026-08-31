import { FingerprintRounded, LoginRounded } from "@mui/icons-material";
import { Box, Button, Typography } from "@mui/material";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import type {
  ProductMe,
  ProductOidcProvider,
  ProductSessionServerPolicy,
} from "./authApi";
import { ProductRecentAuthSheet } from "./ProductRecentAuthSheet";
import {
  productSessionAlertHost,
  subscribeProductSessionAlertHost,
} from "./productSessionAlertHost";
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
  const [desktopFallbackReady, setDesktopFallbackReady] = useState(false);
  const desktopHost = useSyncExternalStore(
    subscribeProductSessionAlertHost,
    productSessionAlertHost,
    productSessionAlertHost,
  );

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
  }, [alertKey]);

  useEffect(() => {
    if (mobile || desktopHost || !alertKey) {
      setDesktopFallbackReady(false);
      return;
    }
    const frame = globalThis.requestAnimationFrame(() =>
      setDesktopFallbackReady(true)
    );
    return () => globalThis.cancelAnimationFrame(frame);
  }, [alertKey, desktopHost, mobile]);

  if (me.auth_enabled === false || !policy || !alert) return null;

  const remaining = Math.max(0, alert.dueAtMs - serverNow);
  const required = alert.phase === "required";
  const urgent = required || remaining <= FINAL_WARNING_MS;
  const sheetOpen = !suspended && (dialogOpen || required);
  const label = sessionCountdownLabel(remaining);
  const title = required
    ? alert.kind === "passkey" ? "Passkey check required" : "Sign-in required"
    : alert.kind === "passkey"
    ? `Passkey check in ${label}`
    : `Sign in again within ${label}`;
  const actionLabel = alert.kind === "passkey" ? "Passkey" : "Sign in";
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

  const reminder = (
    <Button
      data-product-session-alert-button
      data-session-alert-tone={urgent ? "urgent" : "warning"}
      data-desktop-item={!mobile ? "topbar-reauth" : undefined}
      data-desktop-topbar-action={!mobile ? "reauth" : undefined}
      aria-label={`${title}. Open verification`}
      title={title}
      variant="outlined"
      color={urgent ? "error" : "warning"}
      size="small"
      onClick={() => setDialogOpen(true)}
      startIcon={alert.kind === "passkey"
        ? <FingerprintRounded fontSize="small" />
        : <LoginRounded fontSize="small" />}
      sx={{
        pointerEvents: "auto",
        minHeight: mobile ? 44 : undefined,
        maxWidth: mobile ? "min(17rem, calc(100vw - 24px))" : undefined,
        px: mobile ? 1.5 : 0.75,
        borderRadius: mobile ? 999 : undefined,
        bgcolor: "background.paper",
        boxShadow: mobile ? 8 : "none",
        backdropFilter: mobile ? "blur(18px) saturate(75%)" : "none",
        WebkitBackdropFilter: mobile ? "blur(18px) saturate(75%)" : "none",
        textTransform: "none",
        whiteSpace: "nowrap",
        "&:hover": { bgcolor: "background.paper" },
      }}
    >
      <Typography variant="caption" fontWeight={800} noWrap>
        {actionLabel} · {label}
      </Typography>
    </Button>
  );
  const reminderSurface = sheetOpen ? null : mobile
    ? (
      <Box
        sx={{
          position: "fixed",
          top:
            "calc(max(env(safe-area-inset-top, 0px), var(--cowboy-system-top-clearance, 0px)) + 8px)",
          right: 12,
          zIndex: (theme) => theme.zIndex.tooltip + 2,
          pointerEvents: "none",
        }}
      >
        {reminder}
      </Box>
    )
    : desktopHost
    ? createPortal(reminder, desktopHost)
    : desktopFallbackReady
    ? (
      <Box
        sx={{
          position: "fixed",
          top: 12,
          right: 16,
          zIndex: (theme) => theme.zIndex.tooltip + 2,
          pointerEvents: "none",
        }}
      >
        {reminder}
      </Box>
    )
    : null;

  return (
    <>
      {reminderSurface}
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
