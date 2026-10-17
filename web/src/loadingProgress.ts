import type { Components, Theme } from "@mui/material";

// Eight rounded spokes stay distinct in dense rows. The dimmest spoke is
// still visible, so small indicators never collapse into a broken arc.
const spokes = [1, 0.24, 0.32, 0.4, 0.5, 0.62, 0.75, 0.88].map((opacity, index) =>
  `<path d="M12 3.5v3.5" transform="rotate(${index * 45} 12 12)" opacity="${opacity}"/>`
).join("");
const activityMask = `url("data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="white" stroke-width="2.5" stroke-linecap="round">${spokes}</g></svg>`)}")`;

/** Loading glyphs scale with global text on both touch and Desktop. MUI sets
 * size inline, so these two dimensions intentionally override its pixel style.
 * Existing desktopSize callers keep their nominal size but also scale on touch. */
function loadingSize(size: number | string): string {
  if (typeof size === "number") return `${size / 16}rem`;
  if (/^\d+(\.\d+)?px$/.test(size)) return `${Number.parseFloat(size) / 16}rem`;
  return size.replace("var(--cowboy-desktop-size-unit, 16px)", "1rem");
}

/** One motion language for every MUI loading indicator, including lazy
 * surfaces and provider UI. Session dots inherit the root's play state so
 * Mobile's swipe pause also stops their motion. */
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
        // Advance the brightness around fixed spoke positions. No continuously
        // rotating hairline, blur, geometry morphing or per-frame JS work.
        animationDuration: "800ms",
        animationTimingFunction: "steps(8, end)",
        "& .MuiCircularProgress-svg": { display: "none" },
        "&::before": {
          content: '""',
          position: "absolute",
          inset: 0,
          backgroundColor: "currentColor",
          maskImage: activityMask,
          WebkitMaskImage: activityMask,
          maskSize: "100% 100%",
          maskRepeat: "no-repeat",
        },
        // Persistent session activity needs a quiet, legible status mark at
        // toolbar sizes. A crisp solid dot breathes without a blurred halo,
        // gradient or layout movement; its minimum size matches idle.
        '&[data-session-activity="true"]': {
          animation: "none",
          "@keyframes cowboy-session-breathe": {
            "0%, 100%": { opacity: 0.85, transform: "scale(1)" },
            "40%": { opacity: 1, transform: "scale(1.18)" },
            "55%": { opacity: 0.98, transform: "scale(1.16)" },
          },
          "&::before": {
            // 52% of the 1rem slot matches the idle SVG's visible circle.
            inset: "24%",
            borderRadius: "50%",
            maskImage: "none",
            WebkitMaskImage: "none",
            // A gentle crest and longer exhale avoid a mechanical blink.
            animation: "cowboy-session-breathe 2600ms cubic-bezier(0.45, 0, 0.25, 1) infinite",
            animationPlayState: "inherit",
          },
          "@media (prefers-reduced-motion: reduce)": {
            "&::before": { animation: "none" },
          },
          "@media (forced-colors: active)": {
            "&::before": { display: "none" },
          },
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
