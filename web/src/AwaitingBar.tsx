import { Box, IconButton, Stack, Typography, alpha, keyframes } from "@mui/material";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import ArrowUpwardRoundedIcon from "@mui/icons-material/ArrowUpwardRounded";
import { dismissAwaiting } from "./store";

// A slow, subtle hand-wave on the emoji only — reads as "gently waiting" without
// the heavy breathing-border of the old full-width bar.
const wave = keyframes`
  0%, 60%, 100% { transform: rotate(0deg); }
  20% { transform: rotate(14deg); }
  40% { transform: rotate(-8deg); }
`;

// The "agent is waiting for your reply" widget (design §I), as a compact FLOATING
// overlay pill above the composer — it hovers over the transcript instead of a
// full-width bar that pushes layout. Shows whenever confirm-detect judged the last
// turn as awaiting the user (even with an empty queue). Sending a reply clears the
// hold server-side, so this vanishes on its own; the × is the "wasn't a question"
// escape (drains the held queue). When the queue is held it gains a Send-now.
//
// Self-positioning: rendered as the first child of the composer's (relative) box;
// the wrapper is pointer-events:none so transcript scroll passes through the gaps.
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
      sx={{
        position: "absolute",
        left: 0,
        right: 0,
        bottom: "100%",
        display: "flex",
        justifyContent: "center",
        px: 2,
        pb: 1,
        pointerEvents: "none",
        zIndex: 3,
      }}
    >
      <Stack
        role="status"
        direction="row"
        alignItems="center"
        spacing={1}
        onClick={onFocusComposer}
        sx={{
          pointerEvents: "auto",
          cursor: "text",
          maxWidth: "100%",
          pl: 1.5,
          pr: 0.5,
          py: 0.5,
          borderRadius: 999,
          // Frosted, on its own — floats over the transcript, so it carries the
          // blur itself (unlike the in-slab AwaitingBar before).
          bgcolor: (t) => alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.22 : 0.12),
          backdropFilter: "blur(24px) saturate(180%)",
          WebkitBackdropFilter: "blur(24px) saturate(180%)",
          border: (t) => `1px solid ${alpha(t.palette.primary.main, 0.3)}`,
          boxShadow: (t) =>
            `0 6px 20px ${alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.45 : 0.18)}`,
        }}
      >
        <Box
          component="span"
          aria-hidden
          sx={{
            fontSize: "1rem",
            lineHeight: 1,
            transformOrigin: "70% 80%",
            animation: `${wave} 2.6s ease-in-out infinite`,
            "@media (prefers-reduced-motion: reduce)": { animation: "none" },
          }}
        >
          🙋
        </Box>
        <Typography
          variant="body2"
          sx={{ fontWeight: 600, color: "primary.main", whiteSpace: "nowrap" }}
        >
          Waiting for your reply{held ? ` · ${queueLen} held` : ""}
        </Typography>
        {/* One adaptive action. Held → a primary "send the held queue now"; empty
            → a plain dismiss. Both clear the hold (releasing the queue if any) —
            kept as one button so they don't read as two ways to do the same thing.
            (Replying in the composer also clears it, server-side.) */}
        <IconButton
          size="small"
          aria-label={held ? "Send the held queue now" : "Dismiss — the agent wasn't asking"}
          onClick={(e): void => {
            e.stopPropagation();
            dismissAwaiting(sessionId);
          }}
          sx={
            held
              ? {
                  color: "primary.contrastText",
                  bgcolor: "primary.main",
                  "&:hover": { bgcolor: "primary.dark" },
                  width: 28,
                  height: 28,
                }
              : { color: "text.secondary", width: 28, height: 28 }
          }
        >
          {held ? <ArrowUpwardRoundedIcon sx={{ fontSize: 18 }} /> : <CloseRoundedIcon sx={{ fontSize: 18 }} />}
        </IconButton>
      </Stack>
    </Box>
  );
}
