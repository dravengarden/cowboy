export interface TooltipListenerPolicy {
  disableFocusListener: boolean;
  disableTouchListener: boolean;
  disableHoverListener: boolean;
}

/** Tooltips are discoverability affordances for pointers that can really hover. */
export function tooltipListenerPolicy(canHover: boolean): TooltipListenerPolicy {
  return {
    disableFocusListener: true,
    disableTouchListener: true,
    disableHoverListener: !canHover,
  };
}

export function browserTooltipListenerPolicy(): TooltipListenerPolicy {
  return tooltipListenerPolicy(globalThis.matchMedia?.("(hover: hover)").matches ?? false);
}
