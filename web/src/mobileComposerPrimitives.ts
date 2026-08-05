/** Shared geometry for the optional panels stacked above the Mobile composer.
 * Semantic fills may differ (Plan, Queue and Draft communicate different
 * states), but their outer silhouette must read as one component family. */
export const mobileComposerPanelFrameSx = {
  border: 1,
  borderColor: "divider",
  borderRadius: 1,
  overflow: "hidden",
} as const;

/** Matches the compact native composer input's minimum touch height. */
export const mobileComposerPanelHeaderMinHeight = 44;

/** Quiet separation between a focused Mobile composer and the native keyboard. */
export const mobileComposerKeyboardGap = 6;

/** The one outer rhythm shared by Transcript, status, Plan, Pending, and input. */
export const mobileComposerStackGap = 4;

/** One external clearance after whichever row owns the live Transcript tail.
 * Row internals must never manufacture their own boundary exception: Markdown,
 * Thinking, Judging, optimistic messages, and Page footers all terminate above
 * this same spacer. */
export const mobileTranscriptTailGap = 6;

/** One coordinated timeline for the Mobile composer's focus expansion and
 * collapse. Keep adjacent surfaces on this curve so dismissing the keyboard
 * reads as one card settling, rather than several independently clipped rows. */
export const mobileComposerFocusMotion = {
  duration: "240ms",
  easing: "cubic-bezier(.22,1,.36,1)",
} as const;
