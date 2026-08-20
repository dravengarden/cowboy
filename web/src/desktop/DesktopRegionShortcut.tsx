import { Box, type SxProps, type Theme, Tooltip } from "@mui/material";
import type { DesktopPane } from "./DesktopWorkspaceController";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { ShortcutKeycap } from "../ShortcutKeycap";

export function DesktopRegionShortcut({
  shortcut,
  title,
  showWhenPane,
  hideWhenRegion,
  singleKeycap,
  sx,
}: {
  shortcut: string;
  title: string;
  showWhenPane?: DesktopPane;
  hideWhenRegion?: string;
  /** Optional compact label for a global region shortcut rendered as one keycap. */
  singleKeycap?: string;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  if (showWhenPane && workspace.focusedPane !== showWhenPane) return <></>;
  if (hideWhenRegion && workspace.focusedRegion === hideWhenRegion) return <></>;
  return (
    <Tooltip title={`${title} · ${shortcut}`} enterDelay={450}>
      <Box
        component="span"
        data-desktop-region-shortcut
        sx={{ display: "inline-flex", alignItems: "center", flexShrink: 0, ...sx }}
      >
        {singleKeycap && !shortcut.includes(" → ")
          ? <ShortcutKeycap keyLabel={singleKeycap} variant="global" accent />
          : <DesktopShortcut shortcut={shortcut} quiet />}
      </Box>
    </Tooltip>
  );
}
