import { alpha, type Theme } from "@mui/material";
import { mobileComposerPanelFrameSx } from "./mobileComposerPrimitives";

/** The focused Mobile writing material shared by the primary input and
 * Queue/Draft row editors. Keeping it shared prevents pending edits from
 * falling back to a transparent panel that lets transcript text show through.
 * The near-opaque base is intentional: dense transcript glyphs are not a
 * decorative backdrop while the user is writing, even though the surface keeps
 * its blur for the surrounding frosted-material treatment. */
/** Near-opaque paper fill shared by the focused card and its CM6 layer. */
export function mobileFocusedComposerFill(theme: Theme): string {
  return alpha(
    theme.palette.background.paper,
    theme.palette.mode === "dark" ? 0.96 : 0.94,
  );
}

export const mobileFocusedComposerSurfaceSx = {
  borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.42),
  borderRadius: mobileComposerPanelFrameSx.borderRadius,
  bgcolor: mobileFocusedComposerFill,
  backdropFilter: "blur(24px) saturate(140%)",
  WebkitBackdropFilter: "blur(24px) saturate(140%)",
  overflow: "hidden",
  boxShadow: (theme: Theme) =>
    `0 10px 28px ${
      alpha(
        theme.palette.common.black,
        theme.palette.mode === "dark" ? 0.24 : 0.09,
      )
    }`,
} as const;
