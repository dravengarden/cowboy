import { desktopSize } from "../surface/desktopSize";
import CheckIcon from "@mui/icons-material/Check";
import CloudOffOutlinedIcon from "@mui/icons-material/CloudOffOutlined";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import type { ConnectionNotice } from "./connectionNotice";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { DESKTOP_SHORTCUTS } from "./commands/workspaceShortcuts";

/** The Desktop connection notice as painted; the container owns timing. */
export function ConnectionNoticeStrip({
  notice,
  onRetry,
}: {
  notice: ConnectionNotice;
  onRetry: () => void;
}): React.JSX.Element {
  const color = `${notice.tone}.main`;
  return (
    <Box
      role="status"
      aria-live="polite"
      data-desktop-connection-notice={notice.tone}
      sx={{
        mx: 1,
        mt: 0.75,
        px: 1.25,
        py: 0.75,
        display: "flex",
        alignItems: "flex-start",
        gap: 1,
        borderRadius: 1.5,
        border: 1,
        borderColor: (theme) => alpha(theme.palette[notice.tone].main, 0.32),
        bgcolor: (theme) => alpha(theme.palette[notice.tone].main, 0.08),
      }}
    >
      {notice.tone === "success"
        ? <CheckIcon sx={{ fontSize: desktopSize(16), mt: "2px", color }} />
        : <CloudOffOutlinedIcon sx={{ fontSize: desktopSize(16), mt: "2px", color }} />}
      <Stack spacing={0.25} sx={{ minWidth: 0, flex: 1 }}>
        <Typography
          variant="body2"
          sx={{ fontWeight: 650, color, lineHeight: 1.4 }}
        >
          {notice.title}
        </Typography>
        {notice.hint !== null && (
          <Typography
            variant="caption"
            sx={{ color: "text.primary", lineHeight: 1.4 }}
          >
            {notice.hint}
          </Typography>
        )}
        {notice.meta !== null && (
          <Typography
            variant="caption"
            sx={{ color: "text.secondary", lineHeight: 1.4 }}
          >
            {notice.meta}
          </Typography>
        )}
      </Stack>
      {notice.canRetry && (
        <Button
          size="small"
          variant="outlined"
          color="inherit"
          onClick={onRetry}
          data-desktop-sync-retry
          aria-keyshortcuts={DESKTOP_SHORTCUTS.reconnect}
          sx={{
            gap: 0.75,
            flexShrink: 0,
            textTransform: "none",
            fontWeight: 600,
            borderRadius: 999,
            py: 0,
            minHeight: 26,
          }}
        >
          Retry now
          <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.reconnect} quiet />
        </Button>
      )}
    </Box>
  );
}
