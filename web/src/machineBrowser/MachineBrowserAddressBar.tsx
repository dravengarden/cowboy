/** Whose browser on which Machine: the Desktop panel's slim top row and the
 * identity line the phone shows above its bottom toolbar (a phone has no top
 * row, so the framebuffer starts right below the status bar). The remote
 * Chromium shows its own address bar and tabs, so the panel adds none. */
import {
  CloseFullscreenRounded,
  CloseRounded,
  GroupsRounded,
  OpenInFullRounded,
} from "@mui/icons-material";
import { Box, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import type { ReactNode } from "react";

export function Identity({
  accountLabel,
  machineLabel,
  status,
  align,
  shared = false,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  align: "left" | "center";
  /** A shared browser: the group icon and accent mark it. */
  shared?: boolean;
}): React.JSX.Element {
  return (
    <Typography
      variant="caption"
      color="text.secondary"
      noWrap
      title={`${shared ? "Shared browser: " : ""}${accountLabel} · ${machineLabel} · ${status}`}
      data-machine-browser-identity={shared ? "shared" : "own"}
      sx={{ display: "block", minWidth: 0, textAlign: align, lineHeight: "16px" }}
    >
      {shared && (
        <GroupsRounded
          role="img"
          aria-label="Shared browser"
          sx={{ fontSize: "1.2em", mr: 0.5, verticalAlign: "-0.2em", color: "secondary.main" }}
        />
      )}
      <Box
        component="strong"
        sx={{ color: shared ? "secondary.main" : "text.primary", fontWeight: 600 }}
      >
        {accountLabel}
      </Box>
      {` · ${machineLabel} · ${status}`}
    </Typography>
  );
}

/** Desktop: one dense row with identity, the menu, the full-screen toggle
 * and Close. Navigation stays in the remote Chromium's own toolbar. */
export function DesktopBrowserTopBar({
  accountLabel,
  machineLabel,
  status,
  shared = false,
  menu,
  fullscreen,
  onToggleFullscreen,
  onClose,
}: {
  accountLabel: string;
  machineLabel: string;
  status: string;
  shared?: boolean;
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
          shared={shared}
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
