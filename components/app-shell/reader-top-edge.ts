import { alpha, type Theme } from "@mui/material/styles";

// Paint only: no scroll listener, backdrop filter or promoted child layer.
// The host owns the safe-area inset; embedded readers default to zero.
export function readerTopInset(inset?: string): string {
  return `var(--cowboy-reader-top-inset, ${inset ?? "0px"})`;
}

export function readerTopEdge(theme: Theme, inset?: string) {
  const top = readerTopInset(inset);
  const color = theme.palette.background.default;
  return {
    "--cowboy-reader-sticky-top": `calc(${top} + 20px)`,
    "&::after": {
      content: '""', position: "absolute", top: 0, left: 0, right: 0,
      height: `calc(${top} + 20px)`, zIndex: 2, pointerEvents: "none",
      background: `linear-gradient(to bottom, ${color} 0px, ${alpha(color, 0.94)} calc(${top} * 0.55), ${alpha(color, 0.55)} ${top}, ${alpha(color, 0)} 100%)`,
    },
  };
}
