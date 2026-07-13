import { Box, Tooltip } from "@mui/material";
import { isMac } from "../platform";
import { ShortcutKeycap } from "../ShortcutKeycap";

export function desktopRegionShortcut(key: string): string {
  return `Alt+${key}`;
}

function desktopRegionShortcutBadge(key: string): string {
  return `${isMac ? "⌥" : "Alt+"}${key}`;
}

export function DesktopRegionShortcut({
  shortcutKey,
  title,
}: {
  shortcutKey: string;
  title: string;
}): React.JSX.Element {
  const badge = desktopRegionShortcutBadge(shortcutKey);
  return (
    <Tooltip title={`${title} · ${badge}`} enterDelay={450}>
      <Box
        component="span"
        data-desktop-region-shortcut
        sx={{ display: "inline-flex", alignItems: "center", flexShrink: 0 }}
      >
        <ShortcutKeycap keyLabel={badge} variant="global" />
      </Box>
    </Tooltip>
  );
}
