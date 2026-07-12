import { Box, Tooltip } from "@mui/material";
import { isMac } from "../platform";
import { DesktopKeycap } from "./commands/DesktopKeycap";

export function DesktopSessionShortcut({
  digit,
  active,
  title,
}: {
  digit: string;
  active: boolean;
  title: string;
}): React.JSX.Element {
  const modifier = isMac ? "⌘" : "Ctrl+";
  return (
    <Tooltip title={`Switch to ${title} · ${modifier}${digit}`} enterDelay={450}>
      <Box
        component="span"
        className="cowboy-session-shortcut"
        sx={{
          display: "inline-flex",
          opacity: active ? 0.72 : 0.46,
          transition: "opacity 120ms ease",
        }}
      >
        <DesktopKeycap keyLabel={`${modifier}${digit}`} accent={active} quiet />
      </Box>
    </Tooltip>
  );
}
