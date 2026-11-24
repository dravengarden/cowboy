import { useEffect, useReducer } from "react";
import { createPortal } from "react-dom";
import { Box } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { isClippedOutsideScrollContainer } from "./hintTargets";
import { useDesktopHints } from "./leaderContext";

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Paints the armed hint labels over their targets. It is a picture only:
 * no pointer events, no focus, nothing in the accessibility tree; the keys
 * themselves are dispatched by the command provider. Positions follow
 * scrolling and resizing while the labels are up, stay below the leader menu
 * (zIndex modal + 1 < modal + 2), and hide targets scrolled out of view.
 */
export function DesktopHintLayer(): React.JSX.Element | null {
  const hints = useDesktopHints();
  const [, reflow] = useReducer((value: number) => value + 1, 0);
  useEffect(() => {
    if (hints.length === 0) return undefined;
    let frame = 0;
    const schedule = (): void => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(reflow);
    };
    globalThis.addEventListener("scroll", schedule, true);
    globalThis.addEventListener("resize", schedule);
    return () => {
      cancelAnimationFrame(frame);
      globalThis.removeEventListener("scroll", schedule, true);
      globalThis.removeEventListener("resize", schedule);
    };
  }, [hints]);
  if (hints.length === 0 || typeof document === "undefined") return null;
  const maxLeft = Math.max(8, (globalThis.innerWidth || 1024) - 28);
  const maxTop = Math.max(8, (globalThis.innerHeight || 768) - 24);
  return createPortal(
    <Box
      data-desktop-hint-layer
      aria-hidden
      sx={{
        position: "fixed",
        inset: 0,
        pointerEvents: "none",
        zIndex: (theme) => theme.zIndex.modal + 1,
      }}
    >
      {hints.map((hint) => {
        if (!hint.element.isConnected) return null;
        if (isClippedOutsideScrollContainer(hint.element)) return null;
        const rect = hint.element.getBoundingClientRect();
        if (rect.width === 0 && rect.height === 0) return null;
        const style = hint.placement === "row"
          ? {
            left: clamp(rect.left + 4, 8, maxLeft),
            top: clamp(rect.top + rect.height / 2, 12, maxTop),
            transform: "translateY(-50%)",
          }
          : {
            left: clamp(rect.right, 16, maxLeft),
            top: clamp(rect.top, 12, maxTop),
            transform: "translate(-70%, -40%)",
          };
        return (
          <Box
            key={`${hint.label}:${hint.name}`}
            data-desktop-hint={hint.label}
            sx={{ position: "fixed", display: "inline-flex", ...style }}
          >
            <ShortcutKeycap keyLabel={hint.label} variant="context" accent availability="active" />
          </Box>
        );
      })}
    </Box>,
    document.body,
  );
}
