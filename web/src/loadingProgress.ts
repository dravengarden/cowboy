import type { Components, Theme } from "@mui/material";

/** MUI supplies a pixel size inline; normalize it to the global font scale. */
function loadingSize(size: number | string): string {
  if (typeof size === "number") return `${size / 16}rem`;
  if (/^\d+(\.\d+)?px$/.test(size)) return `${Number.parseFloat(size) / 16}rem`;
  return size.replace("var(--cowboy-desktop-size-unit, 16px)", "1rem");
}

function loadingCircumference(thickness: number): number {
  // MUI's circle uses a 44-unit viewBox, inset by half the stroke width.
  return Math.PI * (44 - thickness);
}

// Mobile swipe surfaces keep every ring paint-only so no inner `transform`
// animation promotes a nested compositor layer inside a translated page or
// drawer (docs/mobile-spatial-presentation.md §2.1-2.2).
const MOBILE_SWIPE_RING_SELECTOR = [
  '[data-mobile-product] &:not([data-session-starting="true"])',
  '[data-mobile-drawer-surface] &:not([data-session-starting="true"])',
  '[data-mobile-review-loading] &:not([data-session-starting="true"])',
  '[data-detent-sheet] &:not([data-session-starting="true"])',
  '[data-obsidian-sheet] &:not([data-session-starting="true"])',
].join(", ");

// Agent work and general waits use MUI's native ring; startup keeps its blue bars.
export const loadingProgress: Components<Theme>["MuiCircularProgress"] = {
  defaultProps: { disableShrink: true, enableTrackSlot: true },
  styleOverrides: {
    root: ({ ownerState }) => {
      const circumference = loadingCircumference(ownerState.thickness ?? 3.6);
      return {
        ...(ownerState.variant === "determinate" && {
          "& .MuiCircularProgress-track": { display: "none" },
        }),
        ...(ownerState.variant !== "determinate" && {
          width: `${loadingSize(ownerState.size ?? 40)} !important`,
          height: `${loadingSize(ownerState.size ?? 40)} !important`,
          flexShrink: 0,
          verticalAlign: "middle",
          contain: "layout style",
          // Desktop rotates the square SVG container on the GPU compositor
          // instead of animating strokeDashoffset on the circle (which forces
          // 60fps SVG layout and tile repaints in macOS WebKit) or transforming
          // the child <circle> alone (which WebKit clips to the arc's initial
          // bbox). Mobile swipe surfaces override this to paint-only below.
          animation: "none",
          "@keyframes cowboy-loading-rotate": {
            from: { transform: "rotate(0deg)" },
            to: { transform: "rotate(360deg)" },
          },
          "--cowboy-loading-circumference": `${circumference}px`,
          "@keyframes cowboy-loading-dash": {
            from: { strokeDashoffset: 0 },
            to: { strokeDashoffset: "calc(-1 * var(--cowboy-loading-circumference))" },
          },
          "& .MuiCircularProgress-svg": {
            overflow: "visible",
            transformOrigin: "center",
            animation: "cowboy-loading-rotate 600ms linear infinite",
            animationPlayState: "inherit",
          },
          "& .MuiCircularProgress-circle": {
            animation: "none",
            animationPlayState: "inherit",
            // A one-third arc reads as motion; a longer one looks like a closed ring.
            strokeDasharray: `${circumference / 3}px ${circumference * 2 / 3}px`,
          },
          // A faint full track keeps the slot's footprint without competing with the arc.
          "& .MuiCircularProgress-track": { opacity: 0.2 },
          [MOBILE_SWIPE_RING_SELECTOR]: {
            "& .MuiCircularProgress-svg": {
              animation: "none",
              transform: "none",
              animationPlayState: "inherit",
            },
            "& .MuiCircularProgress-circle": {
              animation: "cowboy-loading-dash 600ms linear infinite",
              animationPlayState: "inherit",
            },
            "@media (prefers-reduced-motion: reduce)": {
              "& .MuiCircularProgress-svg, & .MuiCircularProgress-circle": {
                animation: "none",
              },
            },
          },
          "@media (prefers-reduced-motion: reduce)": {
            animation: "none",
            "& .MuiCircularProgress-svg, & .MuiCircularProgress-circle": {
              animation: "none",
            },
          },
          "@media (forced-colors: active)": {
            color: "CanvasText",
          },
          '&[data-session-starting="true"]': {
            position: "relative",
            animation: "none",
            "@keyframes cowboy-starting-wave": {
              "0%, 60%, 100%": { transform: "scaleY(0.4)" },
              "30%": { transform: "scaleY(1)" },
            },
            "&::before, &::after, & .MuiCircularProgress-svg": {
              position: "absolute",
              top: "18%",
              width: "18%",
              height: "64%",
              borderRadius: "999px",
              backgroundColor: "currentColor",
              transform: "scaleY(0.4)",
              transformOrigin: "center",
              animation: "cowboy-starting-wave 1500ms cubic-bezier(0.4, 0, 0.2, 1) infinite",
              animationPlayState: "inherit",
            },
            "&::before": { content: '""', left: "12%", animationDelay: "-300ms" },
            "& .MuiCircularProgress-svg": { display: "block", left: "41%", animationDelay: "-150ms" },
            "&::after": { content: '""', left: "70%" },
            "& .MuiCircularProgress-circle, & .MuiCircularProgress-track": { display: "none" },
            "@media (prefers-reduced-motion: reduce)": {
              "&::before, &::after, & .MuiCircularProgress-svg": { animation: "none" },
              "& .MuiCircularProgress-svg": { transform: "scaleY(0.8)" },
            },
            "@media (forced-colors: active)": {
              "&::before, &::after, & .MuiCircularProgress-svg": { backgroundColor: "CanvasText", forcedColorAdjust: "none" },
            },
          },
        }),
      };
    },
  },
};
