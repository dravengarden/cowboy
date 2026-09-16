import { keyframes } from "@mui/material";

// Session startup (`status === "starting"`) is a SESSION-level state: the whole
// agent is booting, not one control. So it is presented on session-level
// surfaces only —
//   1. the navbar StatusDot (the persistent per-session token),
//   2. the transcript empty state, which owns the words and carries this bar.
// It is deliberately NOT a spinner inside the composer toolbar: a control-sized
// spinner claims "this button is working", and replacing the primary Send action
// with one removes the very affordance the placeholder invites you to use.
//
// The composer carries no startup mark of its own either. A hairline pinned to
// the card's top edge has to survive the mobile frame's rounded corners and its
// focus/blur transition, and it restates a fact the transcript is already
// animating a few hundred pixels above — one sweep per screen is enough.

/** One indeterminate sweep, for the transcript empty state's startup bar. */
export const prepareSweep = keyframes`
  0% { transform: translateX(-110%); }
  55%, 100% { transform: translateX(310%); }
`;
