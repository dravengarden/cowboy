import { Box, type SxProps, type Theme, Tooltip } from "@mui/material";
import type { DesktopPane } from "./DesktopWorkspaceController";
import { useVimMode } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { ShortcutKeycap } from "../ShortcutKeycap";

export function DesktopRegionShortcut({
  shortcut,
  title,
  normalOnly = false,
  showWhenPane,
  hideWhenRegion,
  availableInComposer = true,
  singleKeycap,
  sx,
}: {
  shortcut: string;
  title: string;
  normalOnly?: boolean;
  showWhenPane?: DesktopPane;
  hideWhenRegion?: string;
  /** Hide a bare region key while the focused editor must retain that key. */
  availableInComposer?: boolean;
  /** Optional compact label for a global region shortcut rendered as one keycap. */
  singleKeycap?: string;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const vimEnabled = useVimSetting();
  const vimMode = useVimMode();
  if (
    normalOnly && workspace.focusedRegion === "prompt.composer" &&
    (!vimEnabled || vimMode !== "normal")
  ) return <></>;
  if (
    workspace.focusedRegion === "prompt.composer" && !availableInComposer
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
        {singleKeycap
          ? <ShortcutKeycap keyLabel={singleKeycap} variant="global" accent />
          : <DesktopShortcut shortcut={shortcut} quiet />}
      </Box>
    </Tooltip>
  );
}
