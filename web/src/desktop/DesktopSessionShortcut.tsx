import { Box, Tooltip } from "@mui/material";
import { isMac } from "../platform";
import { DesktopKeycap } from "./commands/DesktopKeycap";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";

export function DesktopSessionShortcut({
  digit,
  active,
  title,
}: {
  digit: string;
  active: boolean;
  title: string;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const modifier = isMac ? "⌘" : "Ctrl+";
  return (
    <Tooltip title={`Switch to ${title} · ${modifier}${digit}`} enterDelay={450}>
      <Box
        component="span"
        className="cowboy-session-shortcut"
        sx={{
          display: "inline-flex",
          opacity: workspace.focusedRegion === "sessions.list"
            ? (active ? 0.84 : 0.62)
            : 0,
          transition: "opacity 120ms ease",
        }}
      >
        <DesktopKeycap keyLabel={`${modifier}${digit}`} accent={active} quiet />
      </Box>
    </Tooltip>
  );
}
