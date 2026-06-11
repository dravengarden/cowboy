import { useEffect, useRef } from "react";
import { alpha, Box, Button, Stack, Typography } from "@mui/material";
import WarningAmberRounded from "@mui/icons-material/WarningAmberRounded";
import type { RenderItem } from "./derive";
import { send } from "./store";
import { requestStickToBottom, useSticky } from "./stickyStore";
import { frostedPanel, frostedPill } from "./frostedGlass";
import { haptic } from "./haptic";

// The pending tool-permission overlay — a sticky frosted surface floating just
// above the composer, in the SAME slot + material as TurnStatusOverlay (they
// never show at once: a permission request is mid-turn, and the turn-status pill
// hides while the agent is busy). This is the ACTIONABLE surface; the timeline
// keeps only a record marker (PermissionCard).
//
// A tool-approval is high-stakes and BLOCKS the agent, so it stays put:
//   - At the bottom (stuck-to-bottom): EXPANDED — the command verbatim + the
//     full-width option buttons (sentence-case, body-consistent type).
//   - Scrolled up (not stuck): AUTO-COLLAPSES to a compact pill so it never
//     covers history while you read; tapping it scrolls back to the bottom
//     (`requestStickToBottom`), which re-expands it.
//
// Publishes its height into `--awaiting-h` — the var the transcript already
// reserves as padding-bottom for the turn-status pill — so it never paints over
// the last message.
export function PermissionOverlay({
  item,
  sessionId,
}: {
  item: Extract<RenderItem, { kind: "permission" }>;
  sessionId: string;
}): React.JSX.Element {
  // `useSticky` is the existing "is the transcript pinned to the bottom" signal,
  // toggled the instant the user wheels/touches away — so it's exactly the
  // auto-collapse trigger (no new scroll listener on the fragile scroll model).
  const expanded = useSticky(sessionId);
  const measureRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const el = measureRef.current;
    if (!el) return undefined;
    const set = (): void =>
      document.documentElement.style.setProperty(
        "--awaiting-h",
        `${el.offsetHeight}px`,
      );
    set();
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => {
      ro.disconnect();
      document.documentElement.style.setProperty("--awaiting-h", "0px");
    };
  }, [expanded]);

  return (
    <Box
      ref={measureRef}
      sx={{
        position: "absolute",
        left: 0,
        right: 0,
        bottom: 8,
        display: "flex",
        justifyContent: "center",
        px: 2,
        pointerEvents: "none",
        zIndex: 3,
      }}
    >
      {expanded ? (
        <Box
          sx={(t) => ({
            pointerEvents: "auto",
            width: "100%",
            maxWidth: 460,
            p: 1.5,
            borderRadius: 2.5,
            ...frostedPanel(t),
            // A warning tint over the frosted panel (the colour-coded "needs a
            // decision" state, matching the turn-status overlay's amber kinds).
            backgroundImage: `linear-gradient(0deg, ${
              alpha(t.palette.warning.main, t.palette.mode === "dark" ? 0.16 : 0.12)
            }, ${
              alpha(t.palette.warning.main, t.palette.mode === "dark" ? 0.16 : 0.12)
            })`,
          })}
        >
          <Stack
            direction="row"
            spacing={1}
            alignItems="center"
            sx={{ color: "warning.main", mb: 1 }}
          >
            <WarningAmberRounded fontSize="small" sx={{ flexShrink: 0 }} />
            <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
              Permission required
            </Typography>
          </Stack>
          <Box
            component="pre"
            sx={{
              m: 0,
              mb: 1.5,
              px: 1,
              py: 0.75,
              borderRadius: 1,
              bgcolor: "action.hover",
              fontFamily: "ui-monospace, SFMono-Regular, monospace",
              fontSize: 13,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {item.title}
          </Box>
          <Stack spacing={1}>
            {item.options.map((opt) => {
              const reject = opt.kind.startsWith("reject");
              return (
                <Button
                  key={opt.optionId}
                  fullWidth
                  disableElevation
                  variant={reject ? "outlined" : "contained"}
                  color={reject ? "error" : "primary"}
                  onClick={(): void => {
                    haptic();
                    send({
                      type: "permission",
                      session_id: sessionId,
                      request_id: item.requestId,
                      option_id: opt.optionId,
                    });
                  }}
                  sx={{
                    minHeight: { xs: 48, sm: 40 },
                    py: 0.75,
                    textTransform: "none",
                    fontSize: { xs: 15, sm: 14 },
                    lineHeight: 1.35,
                  }}
                >
                  {opt.name}
                </Button>
              );
            })}
          </Stack>
        </Box>
      ) : (
        <Stack
          role="status"
          direction="row"
          alignItems="center"
          spacing={1}
          onClick={(): void => {
            haptic();
            requestStickToBottom(sessionId);
          }}
          sx={(t) => ({
            pointerEvents: "auto",
            cursor: "pointer",
            maxWidth: "100%",
            px: 2,
            py: 0.5,
            minHeight: 36,
            borderRadius: 999,
            userSelect: "none",
            WebkitUserSelect: "none",
            ...frostedPill(t, t.palette.warning.main),
          })}
        >
          <WarningAmberRounded
            sx={{ fontSize: 18, color: "warning.main", flexShrink: 0 }}
          />
          <Typography
            variant="body2"
            sx={{ fontWeight: 600, color: "warning.main", whiteSpace: "nowrap" }}
          >
            Permission required
          </Typography>
        </Stack>
      )}
    </Box>
  );
}
