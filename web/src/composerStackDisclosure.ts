import { keyframes } from "@mui/material";
import { isPairedTouchClick } from "./touchGestures";

// Composer-stack disclosures (Plan, Queue, Drafts, Calls) sit in normal flow
// below the Transcript. Animating MUI Collapse's `height` resized the
// Transcript scroller on every frame of the 300ms transition, so a long
// conversation was laid out again ~18 times per tap: the intermittent
// expand/collapse stutter (Mobile profile: 33–83ms frames at 4× CPU, 17ms with
// a single step). The panel now takes its final height in one layout and only
// the revealed content fades in, which the compositor runs without layout.
export const COMPOSER_STACK_DISCLOSURE_TIMEOUT = 0;

const reveal = keyframes`
  from { opacity: 0; }
  to { opacity: 1; }
`;

/** `sx` for a composer-stack `Collapse`; restarts the fade on each open. */
export function composerStackDisclosureSx(open: boolean) {
  return {
    "& .MuiCollapse-wrapperInner": open
      ? {
        animation: `${reveal} 160ms ease-out`,
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
      }
      : {},
  } as const;
}

const STRAY_CLICK_WINDOW_MS = 800;
let disarmStrayClick: (() => void) | null = null;

/** Arm from a touch-activated disclosure toggle. The panel now changes height
 *  in the same frame as `pointerup`, so WebKit's later compatibility click
 *  lands on whatever moved under the finger: a draft row (opening its edit)
 *  or the Transcript. Swallow exactly that paired click, anywhere; a new
 *  pointerdown, the click itself or a short timeout disarms the guard. */
export function swallowStrayDisclosureClick(): void {
  disarmStrayClick?.();
  const doc = globalThis.document;
  if (doc === undefined) return;
  const onClick = (event: MouseEvent): void => {
    const pointerType = "pointerType" in event
      ? String((event as PointerEvent).pointerType)
      : "";
    disarm();
    if (!isPairedTouchClick(true, event.detail, pointerType)) return;
    event.preventDefault();
    event.stopPropagation();
  };
  const disarm = (): void => {
    doc.removeEventListener("click", onClick, true);
    doc.removeEventListener("pointerdown", disarm, true);
    globalThis.clearTimeout(timer);
    if (disarmStrayClick === disarm) disarmStrayClick = null;
  };
  const timer = globalThis.setTimeout(disarm, STRAY_CLICK_WINDOW_MS);
  doc.addEventListener("click", onClick, true);
  doc.addEventListener("pointerdown", disarm, true);
  disarmStrayClick = disarm;
}
