import { keyframes } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";

// A tool card breathes while its call is in flight. The motion must stay off
// the compositor: iOS WebKit gives an element with a running opacity
// animation its own layer, and inside the column-reverse Transcript scroller
// that layer has been left behind when rows below it changed height (the
// activity row mounting under the card). The card then painted one row lower
// than its layout box, on top of "Analyzing…", with a hole where it belongs
// (the earlier blank-hole fix, 569db9116, was the same layer). A fill colour
// is repainted on the main thread inside the ordinary scroll flow, so it
// cannot be stranded. Do not animate opacity, transform or filter here.
const breath = keyframes`
  0%, 100% { background-color: var(--tool-fill); }
  50%      { background-color: var(--tool-fill-dim); }
`;

/** The card's fill, plus its breathing while `running`. */
export function toolBreathSx(running: boolean, theme: Theme) {
  const fill = theme.palette.background.paper;
  return {
    bgcolor: fill,
    ...(running && {
      "--tool-fill": fill,
      "--tool-fill-dim": alpha(fill, 0.45),
      animation: `${breath} 1.6s ease-in-out infinite`,
    }),
  };
}
