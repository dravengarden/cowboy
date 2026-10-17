import type { PointerEvent as ReactPointerEvent } from "react";

/** Native :active is not reliable on touch when pointer-down preserves an
 * input's focus. Paint the same state explicitly without moving the target or
 * rendering React on pointer movement. Never synthesize a click. */
export function beginDecisionPress(event: ReactPointerEvent): void {
  if (!event.isPrimary || event.button !== 0 || !(event.target instanceof Element)) return;
  const button = event.target.closest<HTMLButtonElement>("button.MuiButton-root");
  if (!button || button.disabled || button.hasAttribute("data-decision-pressed")) return;
  const pointerId = event.pointerId;
  button.setAttribute("data-decision-pressed", "");
  const clear = () => {
    button.removeAttribute("data-decision-pressed");
    document.removeEventListener("pointerup", end, true);
    document.removeEventListener("pointercancel", end, true);
    document.removeEventListener("pointermove", move, true);
    window.removeEventListener("blur", clear);
  };
  const end = (next: PointerEvent) => { if (next.pointerId === pointerId) clear(); };
  const move = (next: PointerEvent) => {
    if (next.pointerId !== pointerId) return;
    const rect = button.getBoundingClientRect();
    if (next.clientX < rect.left || next.clientX > rect.right || next.clientY < rect.top || next.clientY > rect.bottom) clear();
  };
  document.addEventListener("pointerup", end, true);
  document.addEventListener("pointercancel", end, true);
  document.addEventListener("pointermove", move, true);
  window.addEventListener("blur", clear);
}
