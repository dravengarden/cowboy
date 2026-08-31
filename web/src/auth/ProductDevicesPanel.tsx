import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Stack,
  Typography,
} from "@mui/material";
import { ComputerOutlined, DevicesOutlined } from "@mui/icons-material";
import { useCallback, useEffect, useState } from "react";
import { authApi, AuthApiError, type ProductDevice } from "./authApi";
import { useProductAuth } from "./ProductAuthGate";
import { retryWithRecentProductAuth } from "./recentAuth";

function activityLabel(device: ProductDevice): string {
  const timestamp = device.last_used_at_ms ?? device.created_at_ms;
  const label = device.last_used_at_ms ? "Last used" : "Authorized";
  return `${label} ${
    new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(timestamp))
  }`;
}

/** Browser-approved CLI and ACP clients. Browser cookie sessions and Passkeys
 * are separate account resources and intentionally do not appear here. */
export function ProductDevicesPanel({
  hideWhenEmpty = false,
}: {
  hideWhenEmpty?: boolean;
}): React.JSX.Element {
  const { me, reauthenticate } = useProductAuth();
  const [devices, setDevices] = useState<ProductDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [revoking, setRevoking] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (): Promise<void> => {
    const body = await authApi.listDevices();
    setDevices(body.devices);
  }, []);

  useEffect(() => {
    if (me.auth_enabled === false) return;
    setLoading(true);
    void load()
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : "Could not load authorized clients",
        );
      })
      .finally(() => setLoading(false));
  }, [load, me.auth_enabled]);

  const revoke = (device: ProductDevice): void => {
    if (revoking !== null) return;
    setRevoking(device.id);
    setError(null);
    void retryWithRecentProductAuth(
      () => authApi.deleteDevice(device.id),
      reauthenticate,
    )
      .then(load)
      .catch((reason: unknown) => {
        setError(
          reason instanceof AuthApiError
            ? reason.message
            : `Could not revoke ${device.name}`,
        );
      })
      .finally(() => setRevoking(null));
  };

  if (me.auth_enabled === false) return <></>;
  if (
    hideWhenEmpty &&
    (loading || error === null && devices.length === 0)
  ) {
    return <></>;
  }

  return (
    <Stack spacing={2}>
      <Stack direction="row" spacing={1.5} alignItems="center">
        <Box
          sx={{
            width: 44,
            height: 44,
            borderRadius: "50%",
            display: "grid",
            placeItems: "center",
            bgcolor: "action.hover",
            color: "primary.main",
            flex: "0 0 auto",
          }}
        >
          <DevicesOutlined />
        </Box>
        <Box sx={{ minWidth: 0, flex: 1 }}>
          <Typography sx={{ fontWeight: 700 }}>CLI &amp; ACP access</Typography>
          <Typography variant="body2" color="text.secondary">
            Browser-approved client credentials
          </Typography>
        </Box>
        <Chip
          size="small"
          variant="outlined"
          label={`${devices.length} ${
            devices.length === 1 ? "client" : "clients"
          }`}
        />
      </Stack>

      <Typography variant="body2" color="text.secondary">
        CLI and ACP clients request their own revocable credential through
        Cowboy’s normal sign-in page. Browser sessions and Passkeys are managed
        under Session protection and are intentionally separate.
      </Typography>

      {error && <Alert severity="error">{error}</Alert>}
      {loading
        ? (
          <Box sx={{ minHeight: 72, display: "grid", placeItems: "center" }}>
            <CircularProgress size={22} color="inherit" />
          </Box>
        )
        : devices.length === 0
        ? (
          <Box
            sx={{
              border: 1,
              borderColor: "divider",
              borderRadius: 3,
              p: 2,
            }}
          >
            <Typography sx={{ fontWeight: 650 }}>
              No CLI or ACP access yet
            </Typography>
            <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
              Run{" "}
              <code>cowboy login {globalThis.location.origin}</code>, or start a
              Cowboy ACP client, to authorize a separate client.
            </Typography>
          </Box>
        )
        : (
          <Stack spacing={1}>
            {devices.map((device) => (
              <Box
                key={device.id}
                sx={{
                  border: 1,
                  borderColor: "divider",
                  borderRadius: 3,
                  px: 2,
                  py: 1.5,
                }}
              >
                <Stack
                  direction="row"
                  spacing={1.5}
                  alignItems="center"
                  justifyContent="space-between"
                >
                  <Stack
                    direction="row"
                    spacing={1.25}
                    alignItems="center"
                    sx={{ minWidth: 0 }}
                  >
                    <ComputerOutlined color="action" />
                    <Box sx={{ minWidth: 0 }}>
                      <Typography noWrap sx={{ fontWeight: 650 }}>
                        {device.name}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        {activityLabel(device)}
                      </Typography>
                    </Box>
                  </Stack>
                  <Button
                    color="error"
                    size="small"
                    disabled={revoking !== null}
                    onClick={() => revoke(device)}
                    sx={{ flex: "0 0 auto" }}
                  >
                    {revoking === device.id ? "Revoking…" : "Revoke"}
                  </Button>
                </Stack>
              </Box>
            ))}
          </Stack>
        )}
    </Stack>
  );
}
