import { Box, type SxProps, type Theme } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { useDesktopListJumpChord } from "./DesktopCommandProvider";
import { sequentialShortcutAvailability } from "./shortcutAvailability";
import { HintTooltip } from "../../HintTooltip";

/** A list header's `'` label trigger: available while the list owns focus,
 *  active while its row labels are up (FOCUS.md "Labels"). */
export function DesktopListJumpKeycap({
  region,
  keyLabel,
  sx,
}: {
  region: string;
  keyLabel: string;
  prefix?: boolean;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const armed = useDesktopListJumpChord(region);
  const workspace = useDesktopWorkspace();
  const scopeAvailable = workspace.focusedRegion === region;
  const availability = sequentialShortcutAvailability({
    scopeAvailable,
    armed,
    prefix: true,
  });
  return (
    <HintTooltip title={armed
        ? "Press a row's letter to jump to it"
        : `Press ${keyLabel} to label every row with a letter`}>
      <Box
        component="span"
        data-desktop-list-jump-key={keyLabel}
        data-desktop-list-jump-state={availability}
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
          accent={availability !== "inactive"}
          availability={availability}
        />
      </Box>
    </HintTooltip>
  );
}
