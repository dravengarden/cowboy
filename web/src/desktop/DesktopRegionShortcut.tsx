import { Box, type SxProps, type Theme, Tooltip } from "@mui/material";
import type { DesktopPane } from "./DesktopWorkspaceController";
import { useVimMode } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopShortcut } from "./commands/DesktopKeycap";

export function DesktopRegionShortcut({
  shortcut,
  title,
  normalOnly = false,
  showWhenPane,
  hideWhenRegion,
  sx,
}: {
  shortcut: string;
  title: string;
  normalOnly?: boolean;
  showWhenPane?: DesktopPane;
  hideWhenRegion?: string;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const vimEnabled = useVimSetting();
  const vimMode = useVimMode();
  if (
    normalOnly && workspace.focusedRegion === "prompt.composer" &&
    (!vimEnabled || vimMode !== "normal")
  ) return <></>;
  if (showWhenPane && workspace.focusedPane !== showWhenPane) return <></>;
  if (hideWhenRegion && workspace.focusedRegion === hideWhenRegion) return <></>;
  return (
    <Tooltip title={`${title} · ${shortcut}`} enterDelay={450}>
      <Box
        component="span"
        data-desktop-region-shortcut
        sx={{ display: "inline-flex", alignItems: "center", flexShrink: 0, ...sx }}
      >
        <DesktopShortcut shortcut={shortcut} quiet />
      </Box>
    </Tooltip>
  );
}
