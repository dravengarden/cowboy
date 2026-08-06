import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { frostedPill } from "./frostedGlass";
import { TURN_STATUS_PILL_MIN_HEIGHT } from "./floatingOverlayPolicy";

/**
 * Transient judge/transport activity sits at the live Transcript tail rather
 * than in the floating Composer stack.
 * The pill intentionally shares the settled status pill's exact height: when
 * the verdict lands, the transcript row disappears as the actionable Composer
 * status takes over the same visual band without changing the boundary math.
 */
export function TranscriptJudgingActivity(): React.JSX.Element {
  return <TranscriptTurnActivity kind="judging" />;
}

export function TranscriptReconnectingActivity(): React.JSX.Element {
  return <TranscriptTurnActivity kind="reconnecting" />;
}

function TranscriptTurnActivity(
  { kind }: { kind: "judging" | "reconnecting" },
): React.JSX.Element {
  const reconnecting = kind === "reconnecting";
  const label = reconnecting ? "Reconnecting…" : "Judging…";
  const tone = reconnecting ? "warning" : "info";
  return (
    <Box
      data-transcript-turn-activity={kind}
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
        aria-label={label}
        direction="row"
        alignItems="center"
        spacing={1}
        sx={(theme) => ({
          px: 2,
          py: 0.5,
          minHeight: TURN_STATUS_PILL_MIN_HEIGHT,
          borderRadius: 999,
          ...frostedPill(theme, theme.palette[tone].main),
        })}
      >
        <CircularProgress
          size={14}
          thickness={5}
          sx={{ color: `${tone}.main`, flexShrink: 0 }}
        />
        <Typography
          variant="body2"
          sx={{ fontWeight: 600, color: `${tone}.main`, whiteSpace: "nowrap" }}
        >
          {label}
        </Typography>
      </Stack>
    </Box>
  );
}
