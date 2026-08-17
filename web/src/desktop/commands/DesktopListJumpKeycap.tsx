import { Box, type SxProps, type Theme } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { useDesktopListJumpChord } from "./DesktopCommandProvider";
import {
  sequentialShortcutAvailability,
  shortcutAvailability,
} from "./shortcutAvailability";

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
  const workspace = useDesktopWorkspace();
  const scopeAvailable = workspace.focusedRegion === region;
  const availability = prefix
    ? sequentialShortcutAvailability({
      scopeAvailable,
      armed,
      prefix: true,
    })
    : shortcutAvailability(scopeAvailable, armed);
  const title = prefix
    ? (armed ? "Choose 1–9 or 0" : "Press G to reveal direct jump keys")
    : (scopeAvailable
      ? `Jump to item ${keyLabel}`
      : `Focus this list, then press ${keyLabel}`);
  const jumpToItem = (): void => {
    if (!scopeAvailable) return;
    const owner = document.querySelector<HTMLElement>(
      `[data-desktop-region="${CSS.escape(region)}"]`,
    );
    const items = owner
      ? [...owner.querySelectorAll<HTMLElement>("[data-desktop-item]")]
      : [];
    const slot = keyLabel === "0" ? 9 : Number(keyLabel) - 1;
    const target = items[slot];
    if (!target) return;
    target.focus({ preventScroll: true });
    target.scrollIntoView({ block: "nearest" });
  };
  return (
    <Box
      component="span"
      title={title}
      data-desktop-list-jump-key={keyLabel}
      data-desktop-list-jump-state={availability}
      onClick={scopeAvailable && !prefix
        ? (event): void => {
          event.preventDefault();
          event.stopPropagation();
          jumpToItem();
        }
        : undefined}
      sx={[
        {
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
          ...(scopeAvailable && !prefix
            ? { pointerEvents: "auto", cursor: "pointer" }
            : {}),
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
  );
}
