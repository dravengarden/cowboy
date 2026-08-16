import { alpha, type Theme } from "@mui/material";
import { mobileComposerPanelFrameSx } from "./mobileComposerPrimitives";

/** The focused Mobile writing material shared by the primary input and
 * Queue/Draft row editors. Keeping it shared prevents pending edits from
 * falling back to a transparent panel that lets transcript text show through.
 * The fill is fully opaque: iOS composites CodeMirror onto its own layer, so
 * any alpha still samples the transcript and reads as a hole. */
export function mobileFocusedComposerFill(theme: Theme): string {
  return theme.palette.background.paper;
}

export const mobileFocusedComposerSurfaceSx = {
  borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.42),
  borderRadius: mobileComposerPanelFrameSx.borderRadius,
  bgcolor: mobileFocusedComposerFill,
  overflow: "hidden",
  boxShadow: (theme: Theme) =>
    `0 10px 28px ${
      alpha(
        theme.palette.common.black,
        theme.palette.mode === "dark" ? 0.24 : 0.09,
      )
    }`,
} as const;
