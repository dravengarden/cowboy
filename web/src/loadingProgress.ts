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

// Agent work and general waits use MUI's native ring; startup keeps its blue bars.
export const loadingProgress: Components<Theme>["MuiCircularProgress"] = {
  defaultProps: { disableShrink: true, enableTrackSlot: true },
  styleOverrides: {
    root: ({ ownerState }) => ({
      ...(ownerState.variant === "determinate" && {
        "& .MuiCircularProgress-track": { display: "none" },
      }),
      ...(ownerState.variant !== "determinate" && {
        width: `${loadingSize(ownerState.size ?? 40)} !important`,
        height: `${loadingSize(ownerState.size ?? 40)} !important`,
        flexShrink: 0,
        verticalAlign: "middle",
        // Animate paint rather than transforming the SVG circle: WebKit can
        // clip a rotating circle's composited layer in these tiny status slots.
        // The full track and SVG geometry stay on a fixed pixel grid.
        animation: "none",
        "@keyframes cowboy-loading-rotate": {
          from: { strokeDashoffset: 0 },
          to: { strokeDashoffset: "var(--cowboy-loading-offset)" },
        },
        "--cowboy-loading-offset": `${-loadingCircumference(ownerState.thickness ?? 3.6)}px`,
        "& .MuiCircularProgress-svg": { overflow: "visible" },
        "& .MuiCircularProgress-circle": {
          animation: "cowboy-loading-rotate 600ms linear infinite",
          // A one-third arc reads as motion; a longer one looks like a closed ring.
          strokeDasharray: `${loadingCircumference(ownerState.thickness ?? 3.6) / 3}px ${loadingCircumference(ownerState.thickness ?? 3.6) * 2 / 3}px`,
        },
        // A faint full track keeps the slot's footprint without competing with the arc.
        "& .MuiCircularProgress-track": { opacity: 0.2 },
        "& .MuiCircularProgress-svg, & .MuiCircularProgress-circle": {
          animationPlayState: "inherit",
        },
        "@media (prefers-reduced-motion: reduce)": {
          animation: "none",
          "& .MuiCircularProgress-circle": { animation: "none" },
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
    }),
  },
};
