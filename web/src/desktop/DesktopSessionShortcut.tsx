import { Box } from "@mui/material";
import { ShortcutKeycap } from "../ShortcutKeycap";

/**
 * A session row's `␣␣` jump label. It exists only while the switcher layer is
 * armed, so rows stay clean at rest and every label on screen is a key that
 * works right now.
 */
export function DesktopSessionShortcut({ label }: { label: string }): React.JSX.Element {
  return (
    <Box
      component="span"
      className="cowboy-session-shortcut"
      data-session-jump-label={label}
      sx={{ display: "inline-flex", flexShrink: 0 }}
    >
      <ShortcutKeycap keyLabel={label} variant="context" accent availability="active" />
    </Box>
  );
}
