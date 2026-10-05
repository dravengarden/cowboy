import { useEffect, useReducer } from "react";
import { createPortal } from "react-dom";
import { Box } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { useDesktopHints } from "./leaderContext";

/**
 * Paints the armed hint labels over their targets. It is a picture only:
 * no pointer events, no focus, nothing in the accessibility tree; the keys
 * themselves are dispatched by the command provider. Positions follow
 * scrolling and resizing while the labels are up.
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
  return createPortal(
    <Box
      data-desktop-hint-layer
      aria-hidden
      sx={{
        position: "fixed",
        inset: 0,
        pointerEvents: "none",
        zIndex: (theme) => theme.zIndex.tooltip + 1,
      }}
    >
      {hints.map((hint) => {
        if (!hint.element.isConnected) return null;
        const rect = hint.element.getBoundingClientRect();
        if (rect.width === 0 && rect.height === 0) return null;
        const style = hint.placement === "row"
          ? { left: rect.left + 4, top: rect.top + rect.height / 2, transform: "translateY(-50%)" }
          : { left: rect.right, top: rect.top, transform: "translate(-70%, -40%)" };
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
