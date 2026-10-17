import type { Components, Theme } from "@mui/material";

const promptMask = `url("data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 6l6 6-6 6" fill="none" stroke="white" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/></svg>')}")`;

/** MUI supplies a pixel size inline; normalize it to the global font scale. */
function loadingSize(size: number | string): string {
  if (typeof size === "number") return `${size / 16}rem`;
  if (/^\d+(\.\d+)?px$/.test(size)) return `${Number.parseFloat(size) / 16}rem`;
  return size.replace("var(--cowboy-desktop-size-unit, 16px)", "1rem");
}

// General waits use MUI's native ring. Only active agent work gets the >_
// prompt, with a steady chevron and a gently extending cursor.
export const loadingProgress: Components<Theme>["MuiCircularProgress"] = {
  defaultProps: { disableShrink: true },
  styleOverrides: {
    root: ({ ownerState }) => ({
      ...(ownerState.variant !== "determinate" && {
        width: `${loadingSize(ownerState.size ?? 40)} !important`,
        height: `${loadingSize(ownerState.size ?? 40)} !important`,
        flexShrink: 0,
        verticalAlign: "middle",
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
          "& .MuiCircularProgress-circle": { display: "none" },
          "@media (prefers-reduced-motion: reduce)": {
            "&::before, &::after, & .MuiCircularProgress-svg": { animation: "none" },
            "& .MuiCircularProgress-svg": { transform: "scaleY(0.8)" },
          },
          "@media (forced-colors: active)": {
            "&::before, &::after, & .MuiCircularProgress-svg": { backgroundColor: "CanvasText", forcedColorAdjust: "none" },
          },
        },
        '&[data-session-activity="true"]': {
          position: "relative",
          animation: "none",
          "& .MuiCircularProgress-svg": { display: "none" },
          "&::before": {
            content: '""',
            position: "absolute",
            inset: 0,
            backgroundColor: "currentColor",
            maskImage: promptMask,
            WebkitMaskImage: promptMask,
            maskSize: "100% 100%",
            maskRepeat: "no-repeat",
          },
          "&::after": {
            content: '""',
            position: "absolute",
            left: "53%",
            top: "69%",
            width: "38%",
            height: "12.5%",
            borderRadius: "999px",
            backgroundColor: "currentColor",
            transformOrigin: "left center",
            animation: "cowboy-terminal-cursor 1800ms ease-in-out infinite",
            animationPlayState: "inherit",
          },
          "@keyframes cowboy-terminal-cursor": {
            "0%, 100%": { transform: "scaleX(0.75)" },
            "50%": { transform: "scaleX(1)" },
          },
          "@media (prefers-reduced-motion: reduce)": {
            "&::after": { animation: "none" },
          },
          "@media (forced-colors: active)": {
            "&::before, &::after": { backgroundColor: "CanvasText", forcedColorAdjust: "none" },
          },
        },
      }),
    }),
  },
};
