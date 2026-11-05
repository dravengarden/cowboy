/** The Review Markdown preview column: a readable measure with document
 *  heading rules. */
export function markdownPreviewSx(softWrap: boolean) {
  return {
    width: "100%",
    maxWidth: 880,
    mx: "auto",
    px: { xs: 2.25, sm: 4 },
    py: 2.5,
    lineHeight: 1.6,
    overflowWrap: "anywhere",
    "& h1": {
      fontSize: "1.75rem !important",
      pb: 0.5,
      borderBottom: 1,
      borderColor: "divider",
    },
    "& h2": {
      fontSize: "1.4rem !important",
      pb: 0.35,
      borderBottom: 1,
      borderColor: "divider",
    },
    "& h3": { fontSize: "1.18rem !important" },
    "& hr": {
      border: 0,
      borderTop: 1,
      borderColor: "divider",
      my: 2,
    },
    // A sideways-scrolling block is its own scroll container, so
    // WebKit restarts touch-action there and a vertical flick that
    // starts on code or a table would wait for the recognizers
    // again. Declare both native axes on those blocks.
    ...(!softWrap && {
      "& pre, & [data-markdown-table-scroll], & .katex-display": {
        touchAction: "pan-x pan-y pinch-zoom",
      },
    }),
  } as const;
}
