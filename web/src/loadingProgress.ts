import type { Components, Theme } from "@mui/material";

/** Loading glyphs scale with global text on both touch and Desktop. MUI sets
 * size inline, so these two dimensions intentionally override its pixel style.
 * Existing desktopSize callers keep their nominal size but also scale on touch. */
function loadingSize(size: number | string): string {
  if (typeof size === "number") return `${size / 16}rem`;
  if (/^\d+(\.\d+)?px$/.test(size)) return `${Number.parseFloat(size) / 16}rem`;
  return size.replace("var(--cowboy-desktop-size-unit, 16px)", "1rem");
}

/** One motion language for every MUI loading indicator, including lazy
 * surfaces and provider UI. The existing root animation remains the sole
 * moving layer, so Mobile's swipe pause continues to apply. */
export const loadingProgress: Components<Theme>["MuiCircularProgress"] = {
  defaultProps: { disableShrink: true },
  styleOverrides: {
    root: ({ ownerState }) => ({
      ...(ownerState.variant !== "determinate" && {
        width: `${loadingSize(ownerState.size ?? 40)} !important`,
        height: `${loadingSize(ownerState.size ?? 40)} !important`,
        flexShrink: 0,
        verticalAlign: "middle",
        position: "relative",
        // Constant velocity and a fixed silhouette avoid the small-size
        // contracting/expanding arc that looks like a wobbling broken ring.
        animationDuration: "1s",
        animationTimingFunction: "linear",
        "& .MuiCircularProgress-svg": { display: "none" },
        "&::before": {
          content: '""',
          position: "absolute",
          inset: 0,
          borderRadius: "50%",
          background: "conic-gradient(from -90deg, transparent 0deg, currentColor 300deg, transparent 300deg)",
          // A feathered edge stays smooth at fractional font scales. Paint is
          // static; only the containing transform rotates, with no JS timers.
          maskImage: "radial-gradient(closest-side, transparent 76%, #000 80%, #000 94%, transparent 98%)",
          WebkitMaskImage: "radial-gradient(closest-side, transparent 76%, #000 80%, #000 94%, transparent 98%)",
        },
        "@media (prefers-reduced-motion: reduce)": {
          animation: "none",
        },
        // Preserve visibility when system high-contrast suppresses gradients.
        "@media (forced-colors: active)": {
          "&::before": { display: "none" },
          "& .MuiCircularProgress-svg": { display: "block" },
          "& .MuiCircularProgress-circle": {
            animation: "none",
            stroke: "CanvasText",
            strokeWidth: 3,
            strokeLinecap: "round",
            strokeDasharray: "80 126",
          },
        },
      }),
    }),
  },
};
