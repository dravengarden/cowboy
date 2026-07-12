import { Tooltip } from "@mui/material";
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
      <span>
        <DesktopKeycap keyLabel={`${modifier}${digit}`} accent={active} />
      </span>
    </Tooltip>
  );
}
