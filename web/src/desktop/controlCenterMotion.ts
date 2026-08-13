export const CONTROL_CENTER_PANEL_EXIT_MS = 90;
export const CONTROL_CENTER_PANEL_ENTER_MS = 180;

/**
 * Motion stays on the panel content while its scroll-owning shell remains
 * mounted. That keeps the Control Center geometry stable and lets heavy tabs
 * mount offscreen without a blank-frame flash.
 */
export function controlCenterPanelMotionSx(visible: boolean) {
  const duration = visible
    ? CONTROL_CENTER_PANEL_ENTER_MS
    : CONTROL_CENTER_PANEL_EXIT_MS;
  const easing = visible
    ? "cubic-bezier(0.2, 0, 0, 1)"
    : "cubic-bezier(0.4, 0, 1, 1)";
  return {
    minHeight: "100%",
    opacity: visible ? 1 : 0,
    transform: visible ? "translateY(0)" : "translateY(4px)",
    transition:
      `opacity ${String(duration)}ms ${easing}, transform ${String(duration)}ms ${easing}`,
    willChange: "opacity, transform",
    pointerEvents: visible ? "auto" : "none",
    "@media (prefers-reduced-motion: reduce)": {
      transition: "none",
      transform: "none",
    },
  } as const;
}
