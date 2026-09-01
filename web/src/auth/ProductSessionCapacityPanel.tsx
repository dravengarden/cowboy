import DevicesRounded from "@mui/icons-material/DevicesRounded";
import RefreshRounded from "@mui/icons-material/RefreshRounded";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Collapse,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useStoreSelector } from "../store";
import {
  authApi,
  AuthApiError,
  type ProductActiveClient,
  type ProductBrowserSession,
  type ProductSessionInventory,
} from "./authApi";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";

const CARD_SX = {
  border: 1,
  borderColor: "divider",
  borderRadius: 3,
  p: 2,
} as const;

function relativeActivity(timestamp: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - timestamp) / 1_000));
  if (seconds < 60) return "Active now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours}h ago`;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(
    new Date(timestamp),
  );
}

function clientLabel(client: ProductActiveClient): string {
  switch (client.client_kind) {
    case "native_shell":
      return "Native app";
    case "cli":
      return "CLI";
    case "acp":
      return "ACP client";
    default:
      return "Browser";
  }
}

function sessionLabel(session: ProductBrowserSession): string {
  if (session.client_kind === "native_shell") return "Native app session";
  if (session.provider_id) return `${session.provider_id} session`;
  return "Browser session";
}

function compactDuration(milliseconds: number): string {
  if (milliseconds % 60_000 === 0) {
    return `${milliseconds / 60_000}m`;
  }
  return `${Math.round(milliseconds / 1_000)}s`;
}

export function ProductSessionCapacityPanel(): React.JSX.Element {
  const { me, capacity, automation, reauthenticate } = useProductAuth();
  const pushedCapacity = useStoreSelector((state) => state.activeCapacity);
  const [inventory, setInventory] = useState<ProductSessionInventory | null>(
    null,
  );
  const [expanded, setExpanded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (): Promise<void> => {
    try {
      const next = await authApi.listSessions();
      setInventory(next);
      setError(null);
    } catch (reason) {
      setError(
        reason instanceof AuthApiError
          ? reason.message
          : "Could not load session capacity.",
      );
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (me.auth_enabled === false) return;
    void load();
    const refreshVisible = (): void => {
      if (document.visibilityState === "visible") void load();
    };
    document.addEventListener("visibilitychange", refreshVisible);
    return () => document.removeEventListener("visibilitychange", refreshVisible);
  }, [load, me.auth_enabled]);

  const currentSessionId = useMemo(
    () => inventory?.sessions.find((session) => session.current)?.id,
    [inventory],
  );
  const signedIn = inventory?.sessions.length ?? 0;
  const active = inventory?.active_clients.length ??
    pushedCapacity?.active_for_user ?? 0;
  const signedInLimit = capacity?.signed_in_sessions_per_user ??
    inventory?.limit;
  const activeLimit = pushedCapacity?.per_user_limit ??
    capacity?.active_clients_per_user ?? inventory?.active_limit;
  const serviceActive = pushedCapacity?.active_for_service;
  const serviceLimit = pushedCapacity?.service_limit ??
    capacity?.active_clients_service;
  const atCapacity = activeLimit !== undefined && active >= activeLimit;

  const release = (client: ProductActiveClient): void => {
    if (busyId) return;
    setBusyId(client.client_id);
    setError(null);
    void authApi.releaseActiveClient(client.client_id, client.fencing_token)
      .then(load)
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not release the active client.",
        );
      })
      .finally(() => setBusyId(null));
  };

  const revokeSession = (session: ProductBrowserSession): void => {
    if (busyId || session.current) return;
    setBusyId(session.id);
    setError(null);
    void retryWithRecentProductAuth(
      () => authApi.deleteSession(session.id),
      reauthenticate,
    )
      .then(load)
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not sign out that session.",
        );
      })
      .finally(() => setBusyId(null));
  };

  if (me.auth_enabled === false || !capacity) return <></>;

  return (
    <Box sx={CARD_SX} data-product-session-capacity>
      <Stack spacing={1.5}>
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
          spacing={1.25}
        >
          <Stack direction="row" spacing={1.25} alignItems="center" minWidth={0}>
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
              <DevicesRounded />
            </Box>
            <Box minWidth={0}>
              <Typography variant="subtitle1" sx={{ fontWeight: 750 }}>
                Sessions &amp; capacity
              </Typography>
              <Typography variant="body2" color="text.secondary">
                {capacity.enforcement === "enforce"
                  ? "Service limits are enforced"
                  : "Service limits are in observation mode"}
              </Typography>
            </Box>
          </Stack>
          <Chip
            size="small"
            color={atCapacity ? "warning" : "success"}
            variant="outlined"
            label={activeLimit === undefined ? `${active} active` : `${active}/${activeLimit} active`}
          />
        </Stack>

        <Box
          sx={{
            display: "grid",
            gap: 1,
            gridTemplateColumns: { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(3, minmax(0, 1fr))" },
          }}
        >
          {[
            ["Signed-in sessions", signedInLimit === undefined ? signedIn : `${signedIn}/${signedInLimit}`],
            ["Authorized clients", inventory
              ? `${inventory.authorized_clients}/${capacity.authorized_clients_per_user}`
              : `—/${capacity.authorized_clients_per_user}`],
            ["Your active clients", activeLimit === undefined ? active : `${active}/${activeLimit}`],
            ["Service active", serviceActive === undefined || serviceLimit === undefined ? "—" : `${serviceActive}/${serviceLimit}`],
            ["Channels per client", capacity.websocket_channels_per_client],
            ["Automation pool", automation?.enabled ? `${automation.active_clients} seats` : "Disabled"],
          ].map(([label, value]) => (
            <Box key={label} sx={{ bgcolor: "action.hover", borderRadius: 2, minWidth: 0, px: 1.25, py: 1 }}>
              <Typography variant="caption" color="text.secondary">{label}</Typography>
              <Typography variant="body2" sx={{ fontWeight: 750 }}>{value}</Typography>
            </Box>
          ))}
        </Box>

        {pushedCapacity?.status === "waiting" && (
          <Alert severity="warning">
            This view is waiting for an active seat{pushedCapacity.position
              ? ` (queue ${pushedCapacity.position})`
              : ""}. Release one of your other clients below or leave it waiting.
          </Alert>
        )}
        {error && <Alert severity="error">{error}</Alert>}

        <Stack direction="row" spacing={1} justifyContent="space-between">
          <Button
            size="small"
            onClick={() => setExpanded((current) => !current)}
          >
            {expanded ? "Hide details" : "Manage sessions"}
          </Button>
          <Button
            size="small"
            startIcon={loading ? <CircularProgress size={14} /> : <RefreshRounded />}
            disabled={loading || busyId !== null}
            onClick={() => {
              setLoading(true);
              void load();
            }}
          >
            Refresh
          </Button>
        </Stack>

        <Collapse in={expanded} unmountOnExit>
          <Stack spacing={2} sx={{ pt: 0.5 }}>
            <Box>
              <Typography variant="subtitle2" sx={{ fontWeight: 750, mb: 0.5 }}>
                Effective server policy
              </Typography>
              <Typography variant="caption" color="text.secondary" display="block">
                Lease {compactDuration(capacity.active_lease_ms)} · heartbeat {compactDuration(capacity.heartbeat_ms)} · seat reservation {compactDuration(capacity.reservation_ms)}
              </Typography>
              <Typography variant="caption" color="text.secondary" display="block">
                Sessions replace the least recently active session at the limit · active clients wait or reclaim one of their own
              </Typography>
              <Typography variant="caption" color="text.secondary" display="block">
                {capacity.single_session_mode === "newest_wins"
                  ? "Single-session mode: the newest sign-in replaces every older session"
                  : "Single-session mode is off"}
              </Typography>
              <Typography variant="caption" color="text.secondary" display="block">
                {automation?.enabled
                  ? `Automation is enabled in a separate ${automation.active_clients}-client pool; credentials expire within ${compactDuration(automation.credential_max_age_ms)}`
                  : "Automation credentials and their separate client pool are disabled"}
              </Typography>
            </Box>

            <Box>
              <Typography variant="subtitle2" sx={{ fontWeight: 750, mb: 1 }}>
                Active clients
              </Typography>
              <Stack spacing={1}>
                {(inventory?.active_clients ?? []).length === 0
                  ? <Typography variant="body2" color="text.secondary">No active client leases.</Typography>
                  : inventory?.active_clients.map((client) => {
                    const current = client.session_id === currentSessionId;
                    return (
                      <Stack
                        key={`${client.client_id}:${client.fencing_token}`}
                        direction="row"
                        alignItems="center"
                        justifyContent="space-between"
                        spacing={1}
                        sx={{ borderTop: 1, borderColor: "divider", pt: 1 }}
                      >
                        <Box minWidth={0}>
                          <Typography variant="body2" sx={{ fontWeight: 700 }}>
                            {clientLabel(client)}{current ? " · This view" : ""}
                          </Typography>
                          <Typography variant="caption" color="text.secondary">
                            {relativeActivity(client.heartbeat_at_ms)}
                          </Typography>
                        </Box>
                        {!current && (
                          <Button
                            size="small"
                            color="warning"
                            disabled={busyId !== null}
                            onClick={() => release(client)}
                          >
                            {busyId === client.client_id ? "Releasing…" : "Release"}
                          </Button>
                        )}
                      </Stack>
                    );
                  })}
              </Stack>
            </Box>

            <Box>
              <Typography variant="subtitle2" sx={{ fontWeight: 750, mb: 1 }}>
                Signed-in sessions
              </Typography>
              <Stack spacing={1}>
                {(inventory?.sessions ?? []).map((session) => (
                  <Stack
                    key={session.id}
                    direction="row"
                    alignItems="center"
                    justifyContent="space-between"
                    spacing={1}
                    sx={{ borderTop: 1, borderColor: "divider", pt: 1 }}
                  >
                    <Box minWidth={0}>
                      <Typography variant="body2" sx={{ fontWeight: 700 }}>
                        {sessionLabel(session)}{session.current ? " · Current" : ""}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        {relativeActivity(session.last_seen_at_ms)}
                      </Typography>
                    </Box>
                    {!session.current && (
                      <Button
                        size="small"
                        color="error"
                        disabled={busyId !== null}
                        onClick={() => revokeSession(session)}
                      >
                        {busyId === session.id ? "Signing out…" : "Sign out"}
                      </Button>
                    )}
                  </Stack>
                ))}
              </Stack>
            </Box>

          </Stack>
        </Collapse>
      </Stack>
    </Box>
  );
}
