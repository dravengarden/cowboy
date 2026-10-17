import type { Components, Theme } from "@mui/material";
import { loadingWaveAnimation, loadingWaveKeyframes } from "../../components/app-shell/loading-wave";

/** MUI supplies a pixel size inline; normalize it to the global font scale. */
function loadingSize(size: number | string): string {
  if (typeof size === "number") return `${size / 16}rem`;
  if (/^\d+(\.\d+)?px$/.test(size)) return `${Number.parseFloat(size) / 16}rem`;
  return size.replace("var(--cowboy-desktop-size-unit, 16px)", "1rem");
}

// The existing SVG is the middle bar; pseudo-elements supply its neighbours.
// No extra DOM, timers, glow or rotation. Determinate SVGs remain untouched.
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
        animation: "none",
        "@keyframes cowboy-loading-wave": loadingWaveKeyframes,
        "&::before, &::after, & .MuiCircularProgress-svg": {
          position: "absolute",
          top: "18%",
          width: "18%",
          height: "64%",
          borderRadius: "999px",
          backgroundColor: "currentColor",
          transform: "scaleY(0.4)",
          transformOrigin: "center",
          animation: loadingWaveAnimation,
          animationPlayState: "inherit",
        },
        "&::before": { content: '""', left: "12%", animationDelay: "-300ms" },
        "& .MuiCircularProgress-svg": { display: "block", left: "41%", animationDelay: "-150ms" },
        "&::after": { content: '""', left: "70%", animationDelay: "0ms" },
        "& .MuiCircularProgress-circle": { display: "none" },
        "@media (prefers-reduced-motion: reduce)": {
          "&::before, &::after, & .MuiCircularProgress-svg": { animation: "none" },
          "& .MuiCircularProgress-svg": { transform: "scaleY(0.8)" },
        },
        "@media (forced-colors: active)": {
          "&::before, &::after, & .MuiCircularProgress-svg": {
            forcedColorAdjust: "none",
            backgroundColor: "CanvasText",
          },
        },
      }),
    }),
  },
};
