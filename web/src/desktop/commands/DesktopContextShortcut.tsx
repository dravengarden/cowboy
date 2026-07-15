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
  placement = "below",
  children,
}: {
  badge: string;
  shortcut: string;
  showBadge?: boolean;
  placement?: "below" | "corner" | "toolbar";
  children: React.ReactNode;
}): React.JSX.Element {
  const corner = placement === "corner";
  const toolbar = placement === "toolbar";
  const restingTransform = corner
    ? "translate(1px, 1px) scale(.94)"
    : toolbar
    ? "translate(2px, -50%) scale(.94)"
    : "translate(-50%, 2px) scale(.94)";
  const visibleTransform = corner
    ? "translate(0, 0) scale(1)"
    : toolbar
    ? "translate(0, -50%) scale(1)"
    : "translate(-50%, 0) scale(1)";
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
        // Top-bar hints belong beside their control, not below the bar. Keep a
        // narrow inter-control lane for the floating keycap so it never crosses
        // the pane header rail or covers the adjacent action.
        mr: toolbar ? 1.25 : 0,
        "&:hover .cowboy-context-shortcut, &:focus-within .cowboy-context-shortcut":
          {
            opacity: 1,
            transform: visibleTransform,
          },
        "[data-desktop-focused='true'] & .cowboy-context-shortcut": {
          opacity: 0.82,
          transform: visibleTransform,
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
            top: toolbar ? "50%" : "auto",
            bottom: toolbar ? "auto" : corner ? -3 : -5,
            right: corner ? -4 : toolbar ? -9 : "auto",
            left: corner || toolbar ? "auto" : "50%",
            display: "inline-flex",
            opacity: 0,
            transform: restingTransform,
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
