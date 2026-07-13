import { Box } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";

/**
 * A Desktop-only action wrapper. The keycap stays quiet until its owning region
 * is focused, then floats beneath the action without changing toolbar geometry.
 */
export function DesktopContextShortcut({
  badge,
  shortcut,
  showBadge = true,
  children,
}: {
  badge: string;
  shortcut: string;
  showBadge?: boolean;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <Box
      component="span"
      title={shortcut}
      sx={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        height: 40,
        flexShrink: 0,
        "&:hover .cowboy-context-shortcut, &:focus-within .cowboy-context-shortcut":
          {
            opacity: 1,
            transform: "translate(-50%, 0) scale(1)",
          },
        "[data-desktop-focused='true'] & .cowboy-context-shortcut": {
          opacity: 0.82,
          transform: "translate(-50%, 0) scale(1)",
        },
      }}
    >
      {children}
      {showBadge && (
        <Box
          className="cowboy-context-shortcut"
          sx={{
            position: "absolute",
            zIndex: 4,
            bottom: -5,
            left: "50%",
            display: "inline-flex",
            opacity: 0,
            transform: "translate(-50%, 2px) scale(.94)",
            transition: "opacity 120ms ease, transform 120ms ease",
            pointerEvents: "none",
          }}
        >
          <ShortcutKeycap keyLabel={badge} variant="context" />
        </Box>
      )}
    </Box>
  );
}
