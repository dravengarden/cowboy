import { Box } from "@mui/material";
import { useLayoutEffect, useRef, useState } from "react";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { useOptionalDesktopWorkspace } from "../DesktopWorkspaceController";
import { useDesktopListJumpChord } from "./DesktopCommandProvider";
import { shortcutAvailability } from "./shortcutAvailability";

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
  itemScoped = false,
  enabled = true,
  active = false,
  placement = "below",
  children,
}: {
  badge: string;
  shortcut: string;
  showBadge?: boolean;
  /** Keep global shortcuts visible even when their region does not own focus. */
  alwaysVisible?: boolean;
  /** Reveal only for the focused list item, not every row in a focused region. */
  itemScoped?: boolean;
  /** Whether the underlying action can execute in its current business state. */
  enabled?: boolean;
  /** A pending prefix, open overlay, or engaged transient command. */
  active?: boolean;
  placement?: "below" | "corner" | "toolbar" | "inline";
  children: React.ReactNode;
}): React.JSX.Element {
  const corner = placement === "corner";
  const toolbar = placement === "toolbar";
  const inline = placement === "inline";
  const workspace = useOptionalDesktopWorkspace();
  const ownerRef = useRef<HTMLSpanElement>(null);
  const [ownerRegion, setOwnerRegion] = useState<string | null>(null);
  useLayoutEffect(() => {
    setOwnerRegion(
      ownerRef.current?.closest<HTMLElement>("[data-desktop-region]")?.dataset
        .desktopRegion ?? null,
    );
  }, []);
  const listJumpArmed = useDesktopListJumpChord(ownerRegion ?? "");
  const scopeAvailable = enabled && !listJumpArmed && (alwaysVisible ||
    (!!ownerRegion && workspace?.focusedRegion === ownerRegion));
  const availability = shortcutAvailability(scopeAvailable, active);
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
      ref={ownerRef}
      title={shortcut}
      sx={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        height: 40,
        gap: inline ? 0.45 : 0,
        flexShrink: 0,
        // Top-bar hints belong beside their control, not below the bar. Inline
        // keycaps participate in the row, so they cannot cross the pane header
        // rail, cover a neighbour, or get clipped by the toolbar viewport.
        mr: toolbar ? 1.25 : 0,
        ...(!inline && {
          "&:hover .cowboy-context-shortcut, &:focus-within .cowboy-context-shortcut":
            {
              opacity: 1,
              transform: visibleTransform,
            },
        }),
        ...(!itemScoped && {
          "[data-desktop-focused='true'] & .cowboy-context-shortcut": {
            opacity: inline ? 0.78 : 0.82,
            transform: visibleTransform,
          },
        }),
        ...(itemScoped && {
          "[data-desktop-item]:focus & .cowboy-context-shortcut": {
            opacity: inline ? 0.78 : 0.88,
            transform: visibleTransform,
          },
        }),
        ...(alwaysVisible && {
          "& .cowboy-context-shortcut": {
            opacity: 0.72,
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
            left: corner || toolbar ? "auto" : inline ? "auto" : "50%",
            display: "inline-flex",
            overflow: "visible",
            opacity: inline ? 0.72 : itemScoped ? 0 : 0.28,
            transform: restingTransform,
            transition: "opacity 120ms ease, transform 120ms ease",
            pointerEvents: "none",
          }}
        >
          <ShortcutKeycap
            keyLabel={badge}
            variant={inline ? "global" : "context"}
            accent={availability !== "inactive"}
            availability={availability}
          />
        </Box>
      )}
    </Box>
  );
}
