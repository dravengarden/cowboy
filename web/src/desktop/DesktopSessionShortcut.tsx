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
  const modifier = isMac ? "⌥" : "Alt+";
  return (
    <Tooltip title={`Switch to ${title} · ${modifier}${digit}`} enterDelay={450}>
      <Box
        component="span"
        className="cowboy-session-shortcut"
        sx={{
          display: "inline-flex",
          // Alt/Option+1…0 is global navigation, so its affordance must not disappear
          // merely because another pane owns keyboard focus. Focus strengthens
          // the hint; it no longer determines whether the hint exists.
          opacity: active
            ? 0.84
            : workspace.focusedRegion === "sessions.list"
            ? 0.62
            : 0.48,
          transition: "opacity 120ms ease",
        }}
      >
        <DesktopKeycap keyLabel={`${modifier}${digit}`} accent={active} quiet />
      </Box>
    </Tooltip>
  );
}
