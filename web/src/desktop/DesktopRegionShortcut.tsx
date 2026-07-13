import { Box, Tooltip } from "@mui/material";
import { useVimMode } from "../vimModeStore";
import { useVimSetting } from "../vimSetting";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopShortcut } from "./commands/DesktopKeycap";

export function DesktopRegionShortcut({
  shortcut,
  title,
  normalOnly = false,
}: {
  shortcut: string;
  title: string;
  normalOnly?: boolean;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const vimEnabled = useVimSetting();
  const vimMode = useVimMode();
  if (
    normalOnly && workspace.focusedRegion === "prompt.composer" &&
    (!vimEnabled || vimMode !== "normal")
  ) return <></>;
  return (
    <Tooltip title={`${title} · ${shortcut}`} enterDelay={450}>
      <Box
        component="span"
        data-desktop-region-shortcut
        sx={{ display: "inline-flex", alignItems: "center", flexShrink: 0 }}
      >
        <DesktopShortcut shortcut={shortcut} quiet />
      </Box>
    </Tooltip>
  );
}
