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
