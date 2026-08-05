import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { frostedPill } from "./frostedGlass";
import { TURN_STATUS_PILL_MIN_HEIGHT } from "./floatingOverlayPolicy";

/**
 * The asynchronous judge is still part of the turn lifecycle, so its progress
 * sits at the live Transcript tail rather than in the floating Composer stack.
 * The pill intentionally shares the settled status pill's exact height: when
 * the verdict lands, the transcript row disappears as the actionable Composer
 * status takes over the same visual band without changing the boundary math.
 */
export function TranscriptJudgingActivity(): React.JSX.Element {
  return (
    <Box
      data-transcript-turn-activity="judging"
      sx={{
        display: "flex",
        justifyContent: "center",
        width: "100%",
        minWidth: 0,
        px: 2,
        boxSizing: "border-box",
        pointerEvents: "none",
      }}
    >
      <Stack
        role="status"
        aria-label="Judging"
        direction="row"
        alignItems="center"
        spacing={1}
        sx={(theme) => ({
          px: 2,
          py: 0.5,
          minHeight: TURN_STATUS_PILL_MIN_HEIGHT,
          borderRadius: 999,
          ...frostedPill(theme, theme.palette.info.main),
        })}
      >
        <CircularProgress
          size={14}
          thickness={5}
          sx={{ color: "info.main", flexShrink: 0 }}
        />
        <Typography
          variant="body2"
          sx={{ fontWeight: 600, color: "info.main", whiteSpace: "nowrap" }}
        >
          Judging…
        </Typography>
      </Stack>
    </Box>
  );
}
