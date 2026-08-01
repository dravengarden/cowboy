import { Box, type SxProps, type Theme } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { useDesktopListJumpChord } from "./DesktopCommandProvider";

/** Persistent Queue/Draft jump hint with one shared dormant/armed grammar. */
export function DesktopListJumpKeycap({
  region,
  keyLabel,
  prefix = false,
  sx,
}: {
  region: string;
  keyLabel: string;
  prefix?: boolean;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const armed = useDesktopListJumpChord(region);
  const title = prefix
    ? (armed ? "Choose 1–9 or 0" : "Press G to reveal direct jump keys")
    : (armed ? `Jump to item ${keyLabel}` : `Press G, then ${keyLabel}`);
  return (
    <Box
      component="span"
      title={title}
      data-desktop-list-jump-key={keyLabel}
      data-desktop-list-jump-state={armed ? "armed" : "inactive"}
      sx={[
        {
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
        },
        ...(Array.isArray(sx) ? sx : [sx]),
      ]}
    >
      <ShortcutKeycap
        keyLabel={keyLabel}
        variant="context"
        accent={armed}
        availability={armed ? "available" : "inactive"}
      />
    </Box>
  );
}
