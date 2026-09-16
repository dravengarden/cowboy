import type React from "react";
import type { SxProps, Theme } from "@mui/material";
import { Box, keyframes } from "@mui/material";

// Session startup (`status === "starting"`) is a SESSION-level state: the whole
// agent is booting, not one control. So it is presented on session-level
// surfaces only —
//   1. the navbar StatusDot (the persistent per-session token),
//   2. this indeterminate line at the composer card's top edge (the seam between
//      "what the agent has said" and "what you can write" — exactly the boundary
//      that is not ready yet),
//   3. the transcript empty state, which owns the words.
// It is deliberately NOT a spinner inside the composer toolbar: a control-sized
// spinner claims "this button is working", and replacing the primary Send action
// with one removes the very affordance the placeholder invites you to use.
//
// The line is absolutely positioned, so appearing/disappearing costs no layout —
// the composer does not reflow on the ready edge.

/** One indeterminate sweep. Shared by this line and the transcript's empty-state
 *  bar so both startup surfaces move as one signal. */
export const prepareSweep = keyframes`
  0% { transform: translateX(-110%); }
  55%, 100% { transform: translateX(310%); }
`;

/** Full-bleed 2px indeterminate progress line pinned to the top edge of its
 *  (position: relative) parent. Decorative — the accessible announcement belongs
 *  to the status surfaces that carry the wording (the transcript empty state's
 *  `aria-live` region and the StatusDot's label), so this must not add a third
 *  screen-reader voice for the same fact. */
export function SessionPreparingLine(
  { sx }: { sx?: SxProps<Theme> } = {},
): React.JSX.Element {
  return (
    <Box
      aria-hidden
      data-composer-preparing-line
      sx={[
        (theme) => ({
          position: "absolute",
          top: 0,
          left: 0,
          right: 0,
          height: 2,
          zIndex: 7,
          overflow: "hidden",
          pointerEvents: "none",
          // Follow the card's own corner so the line never squares off a rounded
          // composer edge (mobile rounds its frame; desktop is square).
          borderRadius: "inherit",
          bgcolor: theme.palette.action.selected,
          "&::after": {
            content: '""',
            position: "absolute",
            inset: 0,
            width: "34%",
            bgcolor: theme.palette.info.main,
            animation: `${prepareSweep} 1.65s cubic-bezier(.4,0,.2,1) infinite`,
          },
          // A permanently animating hairline is exactly the kind of motion that
          // must yield to the OS preference; keep the affordance as a static
          // tint so the "not ready" boundary is still readable.
          "@media (prefers-reduced-motion: reduce)": {
            "&::after": { animation: "none", width: "100%", opacity: 0.5 },
          },
        }),
        ...(Array.isArray(sx) ? sx : sx ? [sx] : []),
      ]}
    />
  );
}
