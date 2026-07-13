import { Box, Tooltip } from "@mui/material";
import { isMac } from "../platform";
import { ShortcutKeycap } from "../ShortcutKeycap";

export function desktopRegionShortcut(digit: string): string {
  return `${isMac ? "Ctrl" : "Alt"}+${digit}`;
}

function desktopRegionShortcutBadge(digit: string): string {
  return `${isMac ? "⌃" : "Alt+"}${digit}`;
}

export function DesktopRegionShortcut({
  digit,
  title,
}: {
  digit: string;
  title: string;
}): React.JSX.Element {
  const badge = desktopRegionShortcutBadge(digit);
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
