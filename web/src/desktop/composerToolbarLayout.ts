import type { SxProps, Theme } from "@mui/material";

/**
 * The Desktop composer toolbar's responsive layout, by the room its own
 * container has (`composer-tools`): which words, format tiers and keycaps
 * show, and when Compact and Clear fold into one menu.
 */
export const desktopComposerToolbarSx: SxProps<Theme> = {
  containerType: "inline-size",
  containerName: "composer-tools",
  flexShrink: 0,
  minWidth: 0,
  borderTop: 1,
  borderColor: "divider",
  color: "text.secondary",
  "& [data-composer-action-label]": { display: "none" },
  "& [data-format-tier]": { display: "none" },
  "@container composer-tools (min-width: 32rem)": {
    "& [data-format-tier='primary']": { display: "inline-flex" },
  },
  "@container composer-tools (min-width: 42rem)": {
    "& [data-format-tier='secondary']": { display: "inline-flex" },
    "& [data-composer-delivery] [data-composer-action-label]": {
      display: "inline",
    },
  },
  "@container composer-tools (min-width: 56rem)": {
    "& [data-composer-insert] [data-composer-action-label], & [data-composer-action='source'] [data-composer-action-label]":
      { display: "inline" },
  },
  // Compact and Clear fold into one menu where the row cannot hold
  // them on one line; their keys work either way. Their words come
  // last, after Draft, Schedule and Stop have theirs.
  // A narrow pane sheds keycaps before it wraps: first Stop's (the
  // status line shows Mod+. while a turn runs) and the conversation
  // menu's, then Draft's and Schedule's. Keys stay in tooltips and keep working; Send keeps its.
  "& [data-composer-delivery] :is([data-composer-action='stop'], [data-composer-action='conversation']) kbd":
    { display: "none" },
  "& [data-composer-delivery] :is([data-composer-action='draft'], [data-composer-action='schedule']) kbd":
    { display: "none" },
  // Conversation shortcuts read as secondary inline text, not another row
  // of outlined controls. Keep the information visible at every inline width.
  "& [data-composer-delivery] :is([data-composer-action='compact'], [data-composer-action='clear']) kbd":
    {
      display: "inline-flex",
      border: 0,
      bgcolor: "transparent",
      boxShadow: "none",
      px: 0,
      minWidth: 0,
      fontWeight: 500,
      fontFamily: "inherit",
      color: "text.secondary",
      opacity: 0.8,
      "&[data-shortcut-state='inactive']": { opacity: 0.55 },
    },
  // The narrowest row keeps the gauge and drops its number.
  "& [data-session-overflow] [data-context-reading]": { display: "none" },
  "@container composer-tools (min-width: 22rem)": {
    "& [data-session-overflow] [data-context-reading]": {
      display: "inline",
    },
  },
  "@container composer-tools (min-width: 26rem)": {
    "& [data-composer-delivery] :is([data-composer-action='draft'], [data-composer-action='schedule']) kbd":
      { display: "inline-flex" },
  },
  "@container composer-tools (min-width: 28rem)": {
    "& [data-composer-delivery] :is([data-composer-action='stop'], [data-composer-action='conversation']) kbd":
      { display: "inline-flex" },
  },
  "& [data-session-inline]": { display: "none" },
  "& [data-session-overflow]": { display: "inline-flex" },
  "& [data-composer-delivery] [data-session-inline] [data-composer-action-label]":
    { display: "none" },
  "@container composer-tools (min-width: 34rem)": {
    "& [data-session-inline]": { display: "contents" },
    "& [data-session-overflow]": { display: "none" },
  },
  "@container composer-tools (min-width: 60rem)": {
    "& [data-composer-delivery] [data-session-inline] [data-composer-action-label]":
      { display: "inline" },
  },
};
