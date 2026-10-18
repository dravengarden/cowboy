import { Box, Stack } from "@mui/material";
import type { SxProps, Theme } from "@mui/material/styles";
import { type ReactNode, useEffect, useRef } from "react";

/** Keep the capsule material outside the mask: fading the scrollport must not
 * clip its shadow. Edge updates never re-render the document or its editor. */
export function DraftFormatStrip({ children, sx }: {
  children: ReactNode;
  sx: SxProps<Theme>;
}): React.JSX.Element {
  const scroll = useRef<HTMLDivElement>(null);
  const track = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const viewport = scroll.current;
    const content = track.current;
    if (!viewport || !content) return;
    let frame = 0;
    let previous = "";
    const measure = (): void => {
      frame = 0;
      const max = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
      const rtl = getComputedStyle(viewport).direction === "rtl";
      const position = rtl ? max + viewport.scrollLeft : viewport.scrollLeft;
      const left = position > 2;
      const right = position < max - 2;
      const edges = `${left}:${right}`;
      if (edges === previous) return;
      previous = edges;
      viewport.dataset.overflowLeft = String(left);
      viewport.dataset.overflowRight = String(right);
      viewport.parentElement?.style.setProperty(
        "--draft-overflow-left",
        left ? "1" : "0",
      );
      viewport.parentElement?.style.setProperty(
        "--draft-overflow-right",
        right ? "1" : "0",
      );
      const mask = left && right
        ? "linear-gradient(to right, transparent, black 16px, black calc(100% - 16px), transparent)"
        : left
        ? "linear-gradient(to right, transparent, black 16px)"
        : right
        ? "linear-gradient(to right, black calc(100% - 16px), transparent)"
        : "none";
      viewport.style.maskImage = mask;
      viewport.style.webkitMaskImage = mask;
    };
    const schedule = (): void => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    const observer = new ResizeObserver(schedule);
    observer.observe(viewport);
    observer.observe(content);
    viewport.addEventListener("scroll", schedule, { passive: true });
    measure();
    return () => {
      observer.disconnect();
      viewport.removeEventListener("scroll", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }, []);
  return (
    <Box
      sx={[{
        flex: 1,
        minWidth: 0,
        position: "relative",
        "&::before, &::after": {
          content: '""',
          position: "absolute",
          top: "calc(50% - 3px)",
          width: 6,
          height: 6,
          borderTop: "1.5px solid",
          borderRight: "1.5px solid",
          color: "text.secondary",
          pointerEvents: "none",
          zIndex: 1,
        },
        "&::before": {
          left: 5,
          transform: "rotate(-135deg)",
          opacity: "var(--draft-overflow-left, 0)",
        },
        "&::after": {
          right: 5,
          transform: "rotate(45deg)",
          opacity: "var(--draft-overflow-right, 0)",
        },
      }, ...(Array.isArray(sx) ? sx : [sx])]}
    >
      <Box
        ref={scroll}
        data-draft-format-toolbar
        role="group"
        aria-label="Draft formatting"
        sx={{
          overflowX: "auto",
          overscrollBehaviorX: "contain",
          scrollbarWidth: "none",
          borderRadius: "inherit",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        <Stack
          ref={track}
          direction="row"
          alignItems="center"
          sx={{ width: "max-content", px: 0.5 }}
        >
          {children}
        </Stack>
      </Box>
    </Box>
  );
}
