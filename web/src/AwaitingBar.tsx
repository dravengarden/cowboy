import { Box, Button, IconButton, Stack, Typography, alpha, keyframes } from "@mui/material";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import { clearQueue, dismissAwaiting } from "./store";

// A calm, breathing glow so the bar reads as "gently waiting on you" rather than
// an alarm (the Attention-alert handles urgency). Subtle enough to live above the
// composer without nagging.
const breathe = keyframes`
  0%, 100% { box-shadow: 0 0 0 0 rgba(0,0,0,0); }
  50%      { box-shadow: 0 0 0 3px var(--awaiting-glow); }
`;

// The "agent is waiting for your reply" widget (design §I). Shows whenever the
// confirm-detect skill judged the last turn as awaiting the user — EVEN with an
// empty queue. Tapping the body focuses the composer to type a reply. When the
// queue is held it surfaces "· N held" + Send-now / Clear; the × dismisses the
// hold entirely ("the agent wasn't really asking" → the queue drains).
//
// Queued rows remain individually editable in the panel above, so the bar
// intentionally omits a per-row Edit control (it would duplicate that surface).
export function AwaitingBar({
  sessionId,
  queueLen,
  onFocusComposer,
}: {
  sessionId: string;
  queueLen: number;
  onFocusComposer: () => void;
}): React.JSX.Element {
  const held = queueLen > 0;
  return (
    <Box
      role="status"
      onClick={onFocusComposer}
      sx={{
        // Sits on the composer's existing frosted slab — just a tinted pill, no
        // extra blur (would double-frost the seam).
        cursor: "text",
        borderRadius: 2.5,
        px: 1.5,
        py: 1,
        mb: 1,
        border: (t) => `1px solid ${alpha(t.palette.primary.main, 0.35)}`,
        bgcolor: (t) => alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.14 : 0.08),
        "--awaiting-glow": (t) => alpha(t.palette.primary.main, 0.18),
        animation: `${breathe} 3.2s ease-in-out infinite`,
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1}>
        <Box component="span" sx={{ fontSize: "1.05rem", lineHeight: 1 }} aria-hidden>
          🙋
        </Box>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="body2" sx={{ fontWeight: 600, color: "primary.main", lineHeight: 1.3 }}>
            Agent is waiting for your reply
          </Typography>
          {held && (
            <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.3 }}>
              {queueLen} queued message{queueLen > 1 ? "s" : ""} held — won&apos;t auto-send
            </Typography>
          )}
        </Box>
        {held && (
          <Stack direction="row" spacing={0.5} sx={{ flexShrink: 0 }} onClick={(e): void => e.stopPropagation()}>
            <Button
              size="small"
              variant="contained"
              disableElevation
              onClick={(): void => dismissAwaiting(sessionId)}
              sx={{ textTransform: "none", minWidth: 0, px: 1.25 }}
            >
              Send now
            </Button>
            <Button
              size="small"
              color="inherit"
              onClick={(): void => clearQueue(sessionId)}
              sx={{ textTransform: "none", minWidth: 0, px: 1, color: "text.secondary" }}
            >
              Clear
            </Button>
          </Stack>
        )}
        <IconButton
          size="small"
          aria-label="Dismiss — the agent wasn't asking"
          onClick={(e): void => {
            e.stopPropagation();
            dismissAwaiting(sessionId);
          }}
          sx={{ flexShrink: 0, color: "text.secondary" }}
        >
          <CloseRoundedIcon fontSize="small" />
        </IconButton>
      </Stack>
    </Box>
  );
}
