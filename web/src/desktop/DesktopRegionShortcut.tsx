import { Box, Tooltip } from "@mui/material";
import { DesktopShortcut } from "./commands/DesktopKeycap";

export function DesktopRegionShortcut({
  shortcut,
  title,
}: {
  shortcut: string;
  title: string;
}): React.JSX.Element {
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
