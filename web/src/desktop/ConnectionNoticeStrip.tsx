import { desktopSize } from "../surface/desktopSize";
import CheckIcon from "@mui/icons-material/Check";
import CloudOffOutlinedIcon from "@mui/icons-material/CloudOffOutlined";
import { alpha, Box, Button, Typography } from "@mui/material";
import type { ConnectionNotice } from "./connectionNotice";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { DESKTOP_SHORTCUTS } from "./commands/workspaceShortcuts";
import { HintTooltip } from "../HintTooltip";

/**
 * The Desktop connection notice as painted; the container owns timing.
 *
 * It lives inside the Prompt pane header, one line high, so appearing or
 * leaving never moves the editor or its caret. A strip above the editor
 * pushed the text down every time the connection flapped. What to check and
 * the sync details sit in the hover hint.
 */
export function ConnectionNoticeStrip({
  notice,
  onRetry,
}: {
  notice: ConnectionNotice;
  onRetry: () => void;
}): React.JSX.Element {
  const color = `${notice.tone}.main`;
  const detail = notice.hint !== null || notice.meta !== null
    ? (
      <>
        {notice.hint !== null && <Box component="span" sx={{ display: "block" }}>{notice.hint}</Box>}
        {notice.meta !== null && <Box component="span" sx={{ display: "block" }}>{notice.meta}</Box>}
      </>
    )
    : undefined;
  return (
    <Box
      role="status"
      aria-live="polite"
      data-desktop-connection-notice={notice.tone}
      sx={{
        minWidth: 0,
        maxWidth: "100%",
        display: "flex",
        alignItems: "center",
        gap: 0.75,
      }}
    >
      <HintTooltip title={notice.title} detail={detail}>
        <Box
          sx={{
            minWidth: 0,
            flexShrink: 1,
            overflow: "hidden",
            height: 24,
            px: 1,
            display: "flex",
            alignItems: "center",
            gap: 0.75,
            borderRadius: 999,
            border: 1,
            borderColor: (theme) => alpha(theme.palette[notice.tone].main, 0.32),
            bgcolor: (theme) => alpha(theme.palette[notice.tone].main, 0.1),
          }}
        >
          {notice.tone === "success"
            ? <CheckIcon sx={{ fontSize: desktopSize(14), color, flexShrink: 0 }} />
            : <CloudOffOutlinedIcon sx={{ fontSize: desktopSize(14), color, flexShrink: 0 }} />}
          {/* The phase outranks the countdown when the column is narrow. */}
          <Typography
            variant="caption"
            noWrap
            sx={{ fontWeight: 650, color, lineHeight: 1, flexShrink: 1, minWidth: 0 }}
          >
            {notice.title}
          </Typography>
          {notice.countdown !== null && (
            <Typography
              variant="caption"
              noWrap
              sx={{
                color: "text.secondary",
                lineHeight: 1,
                minWidth: 0,
                flexShrink: 1000,
              }}
            >
              {notice.countdown}
            </Typography>
          )}
        </Box>
      </HintTooltip>
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
            minHeight: 24,
            height: 24,
          }}
        >
          Retry now
          <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.reconnect} quiet />
        </Button>
      )}
    </Box>
  );
}
