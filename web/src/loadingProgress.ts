import type { Components, Theme } from "@mui/material";

/** MUI supplies a pixel size inline; normalize it to the global font scale. */
function loadingSize(size: number | string): string {
  if (typeof size === "number") return `${size / 16}rem`;
  if (/^\d+(\.\d+)?px$/.test(size)) return `${Number.parseFloat(size) / 16}rem`;
  return size.replace("var(--cowboy-desktop-size-unit, 16px)", "1rem");
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
        // Keep the full track on a fixed pixel grid; only the bright arc moves.
        animation: "none",
        "@keyframes cowboy-loading-rotate": {
          from: { transform: "rotate(0deg)" },
          to: { transform: "rotate(360deg)" },
        },
        "& .MuiCircularProgress-circle": {
          animation: "cowboy-loading-rotate 1000ms linear infinite",
          transformOrigin: "center",
          transformBox: "fill-box",
        },
        // The full ring must remain readable in the 12–16px status slots.
        "& .MuiCircularProgress-track": { opacity: 0.55 },
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
