import { Box } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";

/**
 * A Desktop-only action wrapper. Pane controls use contextual floating hints;
 * dense toolbars opt into an inline hint that participates in layout and can
 * never overlap a border or neighbouring action.
 */
export function DesktopContextShortcut({
  badge,
  shortcut,
  showBadge = true,
  alwaysVisible = false,
  placement = "below",
  children,
}: {
  badge: string;
  shortcut: string;
  showBadge?: boolean;
  /** Keep global shortcuts visible even when their region does not own focus. */
  alwaysVisible?: boolean;
  placement?: "below" | "corner" | "toolbar" | "inline";
  children: React.ReactNode;
}): React.JSX.Element {
  const corner = placement === "corner";
  const toolbar = placement === "toolbar";
  const inline = placement === "inline";
  const restingTransform = inline
    ? "none"
    : corner
    ? "translate(1px, 1px) scale(.94)"
    : toolbar
    ? "translate(2px, -50%) scale(.94)"
    : "translate(-50%, 2px) scale(.94)";
  const visibleTransform = inline
    ? "none"
    : corner
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
        gap: 0,
        flexShrink: 0,
        // Top-bar hints belong beside their control, not below the bar. Keep a
        // narrow inter-control lane for the floating keycap so it never crosses
        // the pane header rail or covers the adjacent action.
        mr: toolbar ? 1.25 : 0,
        ...(!inline && {
          "&:hover .cowboy-context-shortcut, &:focus-within .cowboy-context-shortcut":
            {
              opacity: 1,
              transform: visibleTransform,
            },
        }),
        "[data-desktop-focused='true'] & .cowboy-context-shortcut": {
          opacity: inline ? 0.72 : 0.82,
          maxWidth: inline ? 32 : undefined,
          ml: inline ? 0.5 : undefined,
          transform: visibleTransform,
        },
        ...(alwaysVisible && {
          "& .cowboy-context-shortcut": {
            opacity: 0.72,
            maxWidth: inline ? 32 : undefined,
            ml: inline ? 0.5 : undefined,
            transform: visibleTransform,
          },
        }),
      }}
    >
      {children}
      {showBadge && (
        <Box
          className="cowboy-context-shortcut"
          sx={{
            position: inline ? "static" : "absolute",
            zIndex: 4,
            top: toolbar ? "50%" : "auto",
            bottom: toolbar ? "auto" : corner ? -3 : -5,
            right: corner ? -4 : toolbar ? -9 : "auto",
            left: corner || toolbar ? "auto" : "50%",
            display: "inline-flex",
            maxWidth: inline ? 0 : undefined,
            ml: 0,
            overflow: inline ? "hidden" : "visible",
            opacity: 0,
            transform: restingTransform,
            transition: inline
              ? "opacity 120ms ease, max-width 120ms ease, margin-left 120ms ease"
              : "opacity 120ms ease, transform 120ms ease",
            pointerEvents: "none",
          }}
        >
          <ShortcutKeycap
            keyLabel={badge}
            variant={inline ? "global" : "context"}
            accent={inline}
          />
        </Box>
      )}
    </Box>
  );
}
