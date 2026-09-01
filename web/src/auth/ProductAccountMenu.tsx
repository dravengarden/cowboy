import LogoutRounded from "@mui/icons-material/LogoutRounded";
import { Box, Button, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";

/** Minimal product sign-out. Lives in auth/ so Desktop/Mobile can call it
 *  without importing store.ts. */
export function ProductAccountMenu(): React.JSX.Element {
  const { me, logout, reauthenticate, signOut } = useProductAuth();
  const [busy, setBusy] = useState<"current" | "provider" | "all" | null>(
    null,
  );
  if (me.auth_enabled === false) return <></>;
  const providerMethod = me.primary_auth_method &&
      me.primary_auth_method !== "password"
    ? me.primary_auth_method
    : null;
  const canLogoutProvider = providerMethod !== null &&
    logout?.provider_logout !== "never";
  const providerLabel = providerMethod === "cardea"
    ? "Cardea"
    : providerMethod;
  const run = (
    scope: "current" | "provider" | "all",
    providerLogout = false,
  ): void => {
    if (busy) return;
    setBusy(scope);
    void retryWithRecentProductAuth(
      () => signOut({ scope, providerLogout }),
      reauthenticate,
      { resumeLabel: scope === "all" ? "Sign out all sessions" : "Sign out" },
    ).finally(() => setBusy(null));
  };
  return (
    <Box
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 3,
        p: 2,
      }}
    >
      <Stack spacing={1.5}>
        <Box>
          <Typography variant="subtitle2" sx={{ fontWeight: 750 }}>
            Sign out
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.4 }}>
            Cowboy removes its session first. Running agents keep working in
            the background.
          </Typography>
        </Box>
        <Button
          color="error"
          variant="outlined"
          size="large"
          fullWidth
          startIcon={<LogoutRounded />}
          disabled={busy !== null}
          onClick={() => run("current")}
        >
          {busy === "current" ? "Signing out…" : "Sign out on this device"}
        </Button>
        {canLogoutProvider && (
          <Button
            color="error"
            variant="text"
            disabled={busy !== null}
            onClick={() => run("provider", true)}
          >
            {busy === "provider"
              ? "Signing out…"
              : `Sign out of Cowboy and ${providerLabel}`}
          </Button>
        )}
        <Button
          color="error"
          variant="text"
          disabled={busy !== null}
          onClick={() => run("all")}
        >
          {busy === "all" ? "Signing out…" : "Sign out all Cowboy sessions"}
        </Button>
        {logout?.backchannel_logout && providerMethod && (
          <Typography variant="caption" color="text.secondary">
            This service accepts signed {providerLabel} logout notifications,
            so provider-initiated sign-out also revokes matching Cowboy
            sessions.
          </Typography>
        )}
      </Stack>
    </Box>
  );
}
