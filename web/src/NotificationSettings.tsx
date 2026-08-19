import {
  CheckCircleOutline,
  NotificationsActiveOutlined,
  NotificationsOffOutlined,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  Divider,
  Stack,
  Switch,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useState } from "react";
import {
  notificationPermissionState,
  disableSystemNotifications,
  presentTestNotification,
  reconcileSystemNotificationSubscription,
  requestSystemNotificationPermission,
  type SessionNotificationCategory,
  updateSystemNotificationPreferences,
  useSystemNotificationPreferences,
} from "./systemNotifications.ts";
import {
  setNotifySetting,
  setVibrateSetting,
  useNotifySetting,
  useVibrateSetting,
} from "./turnNotify.ts";

const CATEGORIES: readonly {
  key: SessionNotificationCategory;
  label: string;
  description: string;
}[] = [
  { key: "completed", label: "Agent finished", description: "A completed answer is ready to review" },
  { key: "input", label: "Needs your input", description: "The agent asks a question or needs a decision" },
  { key: "permission", label: "Permission requests", description: "A tool is waiting for approval" },
  { key: "error", label: "Session errors", description: "A running session cannot continue normally" },
];

function PreferenceRow({
  label,
  description,
  checked,
  disabled = false,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}): React.JSX.Element {
  return (
    <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={2}>
      <Stack sx={{ minWidth: 0 }}>
        <Typography variant="body2">{label}</Typography>
        <Typography variant="caption" color="text.secondary">{description}</Typography>
      </Stack>
      <Switch
        checked={checked}
        disabled={disabled}
        onChange={(event): void => onChange(event.target.checked)}
        inputProps={{ "aria-label": label }}
      />
    </Stack>
  );
}

export function NotificationSettingsContent({ embedded = false }: { embedded?: boolean } = {}): React.JSX.Element {
  const preferences = useSystemNotificationPreferences();
  const sound = useNotifySetting();
  const vibration = useVibrateSetting();
  const [permission, setPermission] = useState(notificationPermissionState);
  const [requesting, setRequesting] = useState(false);
  const [testSent, setTestSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshPermission = useCallback((): void => {
    setPermission(notificationPermissionState());
  }, []);
  useEffect(() => {
    void reconcileSystemNotificationSubscription().then(refreshPermission).catch(() => undefined);
    globalThis.addEventListener("focus", refreshPermission);
    globalThis.document?.addEventListener("visibilitychange", refreshPermission);
    return () => {
      globalThis.removeEventListener("focus", refreshPermission);
      globalThis.document?.removeEventListener("visibilitychange", refreshPermission);
    };
  }, [refreshPermission]);

  const enable = async (): Promise<void> => {
    setRequesting(true);
    setError(null);
    try {
      const result = await requestSystemNotificationPermission();
      setPermission(result);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Notifications could not be enabled.");
    } finally {
      setRequesting(false);
    }
  };
  const enabled = permission === "granted" && preferences.enabled;
  const status = permission === "unsupported"
    ? "Unavailable"
    : permission === "denied"
    ? "Blocked by system"
    : enabled
    ? "Enabled"
    : "Off";

  return (
    <Stack
      spacing={2}
      data-notification-settings="true"
      sx={embedded
        ? { pt: 0, pb: 0 }
        : { pt: { xs: 1.5, md: 0 }, pb: { xs: 12, md: 0 } }}
    >
      <Stack direction="row" alignItems="flex-start" spacing={1.25}>
        <Box sx={{ mt: 0.25, color: enabled ? "success.main" : "text.secondary", display: "grid" }}>
          {enabled ? <NotificationsActiveOutlined /> : <NotificationsOffOutlined />}
        </Box>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="subtitle2">System notifications</Typography>
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", lineHeight: 1.4, pr: 0.5 }}>
            Alerts from the installed Cowboy app when a session needs attention
          </Typography>
        </Box>
        <Chip
          size="small"
          label={status}
          color={enabled ? "success" : "default"}
          variant="outlined"
          sx={{ mt: 0.125, flexShrink: 0 }}
        />
      </Stack>

      {permission === "denied" && (
        <Alert severity="warning">
          Notifications are blocked. Allow Cowboy in iOS Settings or your browser site settings, then return here.
        </Alert>
      )}
      {permission === "unsupported" && (
        <Alert severity="info">
          Install Cowboy to the Home Screen and open the installed app to use Apple notifications.
        </Alert>
      )}
      {error && <Alert severity="error">{error}</Alert>}

      <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
        {!enabled && permission === "default" && (
          <Button variant="contained" disabled={requesting} onClick={() => void enable()}>
            {requesting ? "Requesting…" : "Enable notifications"}
          </Button>
        )}
        {permission === "granted" && (
          <Button
            variant={enabled ? "outlined" : "contained"}
            disabled={requesting}
            onClick={(): void => {
              if (!enabled) {
                void enable();
                return;
              }
              setRequesting(true);
              setError(null);
              void disableSystemNotifications()
                .catch((cause) => setError(cause instanceof Error ? cause.message : "Notifications could not be disabled."))
                .finally(() => setRequesting(false));
            }}
          >
            {enabled ? "Turn off" : "Turn on"}
          </Button>
        )}
        {enabled && (
          <Button
            variant="outlined"
            startIcon={testSent ? <CheckCircleOutline /> : <NotificationsActiveOutlined />}
            onClick={async (): Promise<void> => {
              setTestSent(await presentTestNotification());
              globalThis.setTimeout(() => setTestSent(false), 2000);
            }}
          >
            {testSent ? "Sent" : "Send test"}
          </Button>
        )}
      </Stack>

      <PreferenceRow
        label="Show session names"
        description="Include task names on the Lock Screen; off by default for privacy"
        checked={preferences.showSessionNames}
        disabled={!enabled}
        onChange={(checked): void => updateSystemNotificationPreferences({ showSessionNames: checked })}
      />

      <Divider />
      <Stack spacing={1.75}>
        <Box>
          <Typography variant="overline" color="text.secondary">Notify me when</Typography>
          <Typography variant="caption" color="text.secondary" display="block">
            Visible active sessions stay quiet. Background and other sessions can alert you.
          </Typography>
        </Box>
        {CATEGORIES.map((category) => (
          <PreferenceRow
            key={category.key}
            label={category.label}
            description={category.description}
            checked={preferences.categories[category.key]}
            disabled={!enabled}
            onChange={(checked): void => updateSystemNotificationPreferences({
              categories: { [category.key]: checked },
            })}
          />
        ))}
        <Typography variant="caption" color="text.secondary">
          Mute or unmute an individual session from its ••• menu. System permission always takes priority.
        </Typography>
      </Stack>

      <Divider />
      <Stack spacing={1.75}>
        <Box>
          <Typography variant="overline" color="text.secondary">Delivery</Typography>
          <Typography variant="caption" color="text.secondary" display="block">
            Lock Screen notification sound and vibration follow your device settings.
          </Typography>
        </Box>
        <PreferenceRow
          label="In-app sound"
          description="Play Cowboy's chime when the open app is in the background"
          checked={sound}
          onChange={setNotifySetting}
        />
        <PreferenceRow
          label="In-app vibration"
          description="Use Cowboy haptics when supported by the open app"
          checked={vibration}
          onChange={setVibrateSetting}
        />
      </Stack>

    </Stack>
  );
}
