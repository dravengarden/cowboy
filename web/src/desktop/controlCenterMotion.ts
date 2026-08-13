export const CONTROL_CENTER_PANEL_EXIT_MS = 90;
export const CONTROL_CENTER_PANEL_ENTER_MS = 180;
export const CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME =
  "cowboy-control-center-panel";

export const controlCenterViewTransitionStyles = {
  "@keyframes cowboy-control-center-panel-old": {
    from: { opacity: 1, transform: "translateY(0)" },
    to: { opacity: 0, transform: "translateY(-2px)" },
  },
  "@keyframes cowboy-control-center-panel-new": {
    from: { opacity: 0, transform: "translateY(4px)" },
    to: { opacity: 1, transform: "translateY(0)" },
  },
  [`::view-transition-old(${CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME})`]: {
    animation:
      `cowboy-control-center-panel-old ${String(CONTROL_CENTER_PANEL_ENTER_MS)}ms cubic-bezier(0.4, 0, 0.2, 1) both`,
  },
  [`::view-transition-new(${CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME})`]: {
    animation:
      `cowboy-control-center-panel-new ${String(CONTROL_CENTER_PANEL_ENTER_MS)}ms cubic-bezier(0.2, 0, 0, 1) both`,
  },
} as const;

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
    viewTransitionName: CONTROL_CENTER_PANEL_VIEW_TRANSITION_NAME,
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
