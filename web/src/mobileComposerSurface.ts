import { alpha, type Theme } from "@mui/material";
import { mobileComposerPanelFrameSx } from "./mobileComposerPrimitives";

/** The focused Mobile writing material shared by the primary input and
 * Queue/Draft row editors. Keeping it shared prevents pending edits from
 * falling back to a transparent panel that lets transcript text show through. */
export const mobileFocusedComposerSurfaceSx = {
  borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.42),
  borderRadius: mobileComposerPanelFrameSx.borderRadius,
  bgcolor: (theme: Theme) =>
    alpha(
      theme.palette.background.paper,
      theme.palette.mode === "dark" ? 0.68 : 0.78,
    ),
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
