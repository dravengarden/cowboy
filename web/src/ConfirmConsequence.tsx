import type { ReactNode } from "react";
import { alpha, Box, Typography } from "@mui/material";
import { DESKTOP_INSET_RADIUS } from "./desktop/DesktopEmbeddedControl";

/**
 * The concrete effect of a confirmation, as a block instead of an orphan line.
 *
 * A confirm card says two different things: what the action MEANS (the
 * paragraph) and what happens to THIS session the moment you tap (the provider,
 * the running turn, the irreversibility). The second one used to render as a
 * 13px grey sentence floating between the paragraph and the buttons, which read
 * as leftover text rather than the last thing to check before committing.
 *
 * Here it is one tinted block in the decision's own colour, sitting directly
 * above the action bar — the colour is the same one the confirm button is about
 * to use, so the warning is stated twice in the same hue rather than once in
 * grey.
 */
export function ConfirmConsequence({
  tone = "primary",
  children,
  irreversible,
}: {
  readonly tone?: "primary" | "error";
  readonly children: ReactNode;
  /** Pulled out of the body copy on purpose: "this can't be undone" is the one
   *  sentence that must not be the tail of a three-line paragraph. */
  readonly irreversible?: boolean | undefined;
}): React.JSX.Element {
  return (
    <Box
      data-confirm-consequence={tone}
      sx={(theme) => {
        const color = theme.palette[tone].main;
        return {
          mt: 1.75,
          px: 1.25,
          py: 0.9,
          borderRadius: `${DESKTOP_INSET_RADIUS}px`,
          border: `1px solid ${
            alpha(color, theme.palette.mode === "dark" ? 0.32 : 0.24)
          }`,
          backgroundColor: alpha(
            color,
            theme.palette.mode === "dark" ? 0.12 : 0.07,
          ),
        };
      }}
    >
      <Typography
        variant="body2"
        sx={{ lineHeight: 1.5, color: "text.primary" }}
      >
        {children}
        {irreversible && (
          <Box
            component="span"
            sx={{
              display: "block",
              mt: 0.35,
              fontWeight: 750,
              color: `${tone}.main`,
            }}
          >
            This can't be undone.
          </Box>
        )}
      </Typography>
    </Box>
  );
}
