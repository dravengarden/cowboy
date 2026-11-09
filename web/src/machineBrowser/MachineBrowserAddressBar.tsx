/** The top of the Machine browser panel: whose browser on which Machine, in
 * one slim row. The remote Chromium shows its own address bar and tabs, so
 * the panel adds none. */
import { CloseFullscreenRounded, CloseRounded, OpenInFullRounded } from "@mui/icons-material";
import { Box, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import type { ReactNode } from "react";

function Identity({
  accountLabel,
  machineLabel,
  status,
  align,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  align: "left" | "center";
}): React.JSX.Element {
  return (
    <Typography
      variant="caption"
      color="text.secondary"
      noWrap
      title={`${accountLabel} · ${machineLabel} · ${status}`}
      sx={{ display: "block", minWidth: 0, textAlign: align, lineHeight: 1.4 }}
    >
      <Box component="strong" sx={{ color: "text.primary", fontWeight: 600 }}>
        {accountLabel}
      </Box>
      {` · ${machineLabel} · ${status}`}
    </Typography>
  );
}

/** Phone: Close and the identity line in one slim row, clearing the status
 * bar and side safe areas. */
export function MobileBrowserTopBar({
  accountLabel,
  machineLabel,
  status,
  onClose,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      data-machine-browser-topbar
      sx={{
        flexShrink: 0,
        position: "relative",
        zIndex: 2,
        borderBottom: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        pt: "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
        pl: "max(4px, env(safe-area-inset-left, 0px))",
        pr: "max(12px, env(safe-area-inset-right, 0px))",
      }}
    >
      <IconButton aria-label="Close browser" onClick={onClose} sx={{ width: 40, height: 40 }}>
        <CloseRounded fontSize="small" />
      </IconButton>
      <Box sx={{ flex: 1, minWidth: 0, pr: 5 }}>
        <Identity
          accountLabel={accountLabel}
          machineLabel={machineLabel}
          status={status}
          align="center"
        />
      </Box>
    </Stack>
  );
}

/** Desktop: one dense row with identity, the menu, the full-screen toggle
 * and Close. Navigation stays in the remote Chromium's own toolbar. */
export function DesktopBrowserTopBar({
  accountLabel,
  machineLabel,
  status,
  menu,
  fullscreen,
  onToggleFullscreen,
  onClose,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  menu: ReactNode;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
  onClose: () => void;
}): React.JSX.Element {
  const fullLabel = fullscreen ? "Exit full screen" : "Full screen";
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.5}
      data-machine-browser-topbar
      sx={{ flexShrink: 0, position: "relative", zIndex: 2, px: 1, py: 0.75 }}
    >
      <Box sx={{ flex: 1, minWidth: 0, px: 1 }}>
        <Identity
          accountLabel={accountLabel}
          machineLabel={machineLabel}
          status={status}
          align="left"
        />
      </Box>
      {menu}
      <Tooltip title={fullscreen ? "Exit full screen (Esc)" : "Full screen"}>
        <IconButton
          aria-label={fullLabel}
          aria-pressed={fullscreen}
          onClick={onToggleFullscreen}
          sx={{ width: 34, height: 34 }}
        >
          {fullscreen ? <CloseFullscreenRounded /> : <OpenInFullRounded />}
        </IconButton>
      </Tooltip>
      <Tooltip title="Close">
        <IconButton aria-label="Close browser" onClick={onClose} sx={{ width: 34, height: 34 }}>
          <CloseRounded />
        </IconButton>
      </Tooltip>
    </Stack>
  );
}
