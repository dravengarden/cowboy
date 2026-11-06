import { alpha, type Theme } from "@mui/material/styles";

/** One bounded frost layer behind floating controls, never on scrolling content. */
export function readerBottomEdge(theme: Theme) {
  const color = theme.palette.background.default;
  const mask = "linear-gradient(to bottom, transparent, #000 56px)";
  return {
    isolation: "isolate",
    "&::before": {
      content: '""', position: "absolute", inset: "-32px 0 0", zIndex: -1,
      pointerEvents: "none", backdropFilter: "blur(8px)", WebkitBackdropFilter: "blur(8px)",
      maskImage: mask, WebkitMaskImage: mask,
      background: `linear-gradient(to bottom, ${alpha(color, 0)}, ${alpha(color, 0.8)} 64px, ${color})`,
      "@media (prefers-reduced-transparency: reduce)": { backdropFilter: "none", WebkitBackdropFilter: "none", background: color },
    },
  };
}
