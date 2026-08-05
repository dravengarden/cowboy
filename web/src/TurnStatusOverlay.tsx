import { useEffect, useRef, useState } from "react";
import { alpha, Box, CircularProgress, IconButton, Stack, Typography } from "@mui/material";
import type { PaletteColor, Theme } from "@mui/material";
import ArrowUpwardRounded from "@mui/icons-material/ArrowUpwardRounded";
import ExpandMoreRounded from "@mui/icons-material/ExpandMoreRounded";
import type { Status } from "./protocol";
import {
  dismissAwaiting,
  resumeTurn,
  retryTurn,
  setPaused,
  useConnected,
  useJudgeResult,
} from "./store";
import { openJudgeInspector } from "./JudgeInspector";
import { confirmationHaptic } from "./haptic";
import { frostedPill } from "./frostedGlass";
import { TURN_STATUS_PILL_MIN_HEIGHT } from "./floatingOverlayPolicy";
import {
  deriveTurnStatusKind,
  type TurnStatusKind,
} from "./turnStatusPolicy";

type PaletteKey = "primary" | "success" | "warning" | "error" | "info";

// The settled/actionable half of turn status. Transient judge progress belongs
// to the live Transcript tail; this Composer-owned pill appears only when the
// verdict or session state leaves the user something persistent to understand
// or act on.
const KIND_META: Record<TurnStatusKind, { color: PaletteKey; label: string }> = {
  offline: { color: "warning", label: "Reconnecting…" },
  awaiting: { color: "primary", label: "Waiting for your reply" },
  done: { color: "success", label: "Task complete" },
  interrupted: { color: "warning", label: "Turn interrupted" },
  paused: { color: "warning", label: "Queue paused" },
  error: { color: "error", label: "Agent error" },
};

export function TurnStatusOverlay({
  sessionId,
  status,
  working,
  judging,
  awaitingUser,
  done,
  paused,
  queue,
  onFocusComposer,
}: {
  sessionId: string;
  status: Status;
  /** The agent is actively working == the ACP turn is in flight (Zed's
   *  `Generating`). The whole overlay is hidden while true — the working spinner
   *  owns the slot, so a "Queue paused / done / …" pill never shows next to it. */
  working: boolean;
  judging: boolean;
  awaitingUser: boolean;
  done: boolean;
  paused: boolean;
  queue: { id: string }[];
  onFocusComposer: () => void;
}): React.JSX.Element | null {
  // Connection loss → a debounced "Reconnecting…" pill. The 600ms debounce keeps a
  // sub-second blip (the common case on flaky cellular) from flashing the pill; a
  // real outage still escalates to the shared red ConnectionBanner up top.
  const connected = useConnected();
  const [offline, setOffline] = useState(false);
  useEffect(() => {
    if (connected) {
      setOffline(false);
      return undefined;
    }
    const t = globalThis.setTimeout(() => setOffline(true), 600);
    return () => globalThis.clearTimeout(t);
  }, [connected]);

  const kind = deriveTurnStatusKind({
    offline,
    status,
    working,
    judging,
    awaitingUser,
    done,
    paused,
  });
  const judge = useJudgeResult(sessionId);
  const [expanded, setExpanded] = useState(false);

  // Long-press the pill → open the judge-run inspector (full history + raw I/O +
  // delete). A press that lands on a control (chevron / action / ×) is ignored so
  // those keep their own tap. `fired` suppresses the trailing click (focus).
  const lpTimer = useRef<number | null>(null);
  const lpFired = useRef(false);
  // `pressing` drives the press-and-hold GROW animation: the pill swells while
  // held (a 480ms transition matching the threshold), so the hold reads as
  // "building up", then snaps back as the inspector opens.
  const [pressing, setPressing] = useState(false);
  const clearLongPress = (): void => {
    if (lpTimer.current !== null) {
      clearTimeout(lpTimer.current);
      lpTimer.current = null;
    }
    setPressing(false);
  };
  const startLongPress = (e: React.PointerEvent): void => {
    if ((e.target as HTMLElement).closest("button")) return;
    lpFired.current = false;
    clearLongPress();
    setPressing(true);
    lpTimer.current = globalThis.setTimeout(() => {
      lpFired.current = true;
      setPressing(false);
      // A firmer tap right as the inspector opens — the "you got it" confirmation.
      confirmationHaptic();
      openJudgeInspector(sessionId);
    }, 480);
  };
  useEffect(() => clearLongPress, []);

  // A new state collapses the previous judge detail. The completed state remains
  // visible until the next submit clears it in the server or another state
  // supersedes it, so returning to a finished session still communicates the
  // judge's result.
  useEffect(() => {
    setExpanded(false);
  }, [kind]);

  if (kind === null) return null;
  const meta = KIND_META[kind];
  const held = queue.length > 0;
  const tone = (t: Theme): PaletteColor => t.palette[meta.color];
  const showExpand = (kind === "awaiting" || kind === "done") &&
    judge !== undefined;

  // The primary trailing action (besides the × dismiss), per kind.
  let action:
    | { label: string; icon?: React.JSX.Element; onClick: () => void }
    | undefined;
  if (kind === "awaiting" && held) {
    action = {
      label: "Send",
      icon: <ArrowUpwardRounded sx={{ fontSize: 18 }} />,
      onClick: () => dismissAwaiting(sessionId),
    };
  } else if (kind === "interrupted") {
    action = { label: "Resume", onClick: () => resumeTurn(sessionId) };
  } else if (kind === "paused") {
    // Release the manual queue pause — the queue drain picks back up (waiting for
    // any in-flight turn to end first). Mirrors the interrupted "Resume".
    action = { label: "Resume", onClick: () => setPaused(sessionId, false) };
  } else if (kind === "error") {
    action = { label: "Retry", onClick: () => retryTurn(sessionId) };
  }
  // Symmetric padding (centred text) when there's no trailing control; otherwise
  // tighten the right so the button sits in.
  const hasTrailing = showExpand || action !== undefined;

  return (
    <Box
      data-turn-status-overlay
      data-composer-stack-slot="status"
      sx={{
        position: "relative",
        display: "flex",
        justifyContent: "center",
        px: 2,
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
        pointerEvents: "none",
        zIndex: 3,
      }}
    >
      <Stack
        alignItems="center"
        spacing={0.75}
        sx={{ maxWidth: "100%", pointerEvents: "auto" }}
      >
        {expanded && judge && (
          <Box
            sx={{
              width: "100%",
              maxWidth: 460,
              p: 1.25,
              borderRadius: 2.5,
              // Same liquid-glass material as the pill, a touch more base since the
              // raw text is denser. Glass edge via inset highlights, no hard border.
              backgroundColor: (t) =>
                alpha(
                  t.palette.background.default,
                  t.palette.mode === "dark" ? 0.46 : 0.54,
                ),
              backdropFilter: "blur(40px) saturate(180%) brightness(1.06)",
              WebkitBackdropFilter:
                "blur(40px) saturate(180%) brightness(1.06)",
              boxShadow: (t) =>
                `0 8px 28px ${
                  alpha(
                    t.palette.common.black,
                    t.palette.mode === "dark" ? 0.5 : 0.18,
                  )
                }`,
              fontSize: 11,
              maxHeight: 280,
              overflow: "auto",
            }}
          >
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ display: "block", mb: 0.5 }}
            >
              {judge.layer} · {judge.model || "no model"} ·{" "}
              {judge.latency_ms}ms · cache hit {judge.cache_hit}/miss{" "}
              {judge.cache_miss} · conf {judge.confidence.toFixed(2)}
            </Typography>
            <Typography variant="caption" sx={{ fontWeight: 600 }}>
              Input
            </Typography>
            <Box
              component="pre"
              sx={{
                m: 0,
                mb: 0.75,
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                fontSize: 11,
              }}
            >
              {judge.input || "—"}
            </Box>
            <Typography variant="caption" sx={{ fontWeight: 600 }}>
              Output
            </Typography>
            <Box
              component="pre"
              sx={{
                m: 0,
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                fontSize: 11,
              }}
            >
              {judge.output || "—"}
            </Box>
          </Box>
        )}
        <Stack
          role="status"
          data-turn-status-pill
          direction="row"
          alignItems="center"
          spacing={1}
          onPointerDown={startLongPress}
          onPointerUp={clearLongPress}
          onPointerLeave={clearLongPress}
          onPointerCancel={clearLongPress}
          onClick={(): void => {
            // A long-press already opened the inspector — swallow the trailing
            // click so it doesn't also focus the composer.
            if (lpFired.current) {
              lpFired.current = false;
              return;
            }
            onFocusComposer();
          }}
          sx={(t) => ({
            cursor: "text",
            maxWidth: "100%",
            // Press-and-hold grows the pill; release/settle snaps it back.
            transform: pressing ? "scale(1.07)" : "scale(1)",
            transition: "transform .42s cubic-bezier(.4,0,.2,1)",
            // A long-press must NOT trigger the OS text-selection (the blue
            // handles) on the label — it's a gesture target, not copyable text.
            userSelect: "none",
            WebkitUserSelect: "none",
            WebkitTouchCallout: "none",
            ...(hasTrailing ? { pl: 2, pr: 0.5 } : { px: 2 }),
            // Symmetric, tight vertical padding so a trailing pill button (Retry /
            // Resume / Send) sits with a uniform ~4px inset instead of the old
            // lopsided pt0.75/pb1 gap that left it floating with ugly margin.
            py: 0.5,
            minHeight: TURN_STATUS_PILL_MIN_HEIGHT,
            borderRadius: 999,
            // Shared with PermissionOverlay so every occupant of this floating
            // slot has the same glass depth and boundary-safe elevation.
            ...frostedPill(t, tone(t).main),
          })}
        >
          {kind === "offline" && (
            <CircularProgress
              size={14}
              thickness={5}
              sx={{ color: (t) => tone(t).main, flexShrink: 0 }}
            />
          )}
          <Typography
            variant="body2"
            sx={{
              fontWeight: 600,
              color: (t) => tone(t).main,
              whiteSpace: "nowrap",
            }}
          >
            {meta.label}
            {kind === "awaiting" && held ? ` · ${queue.length} held` : ""}
          </Typography>
          {showExpand && (
            <IconButton
              size="small"
              aria-label={expanded ? "Hide judge detail" : "Show judge detail"}
              onClick={(e): void => {
                e.stopPropagation();
                setExpanded((v) => !v);
              }}
              sx={{ color: "text.secondary", width: 26, height: 26 }}
            >
              <ExpandMoreRounded
                sx={{
                  fontSize: 18,
                  transform: expanded ? "rotate(180deg)" : "none",
                  transition: "transform .15s",
                }}
              />
            </IconButton>
          )}
          {action && (
            <IconButton
              size="small"
              aria-label={action.label}
              onClick={(e): void => {
                e.stopPropagation();
                action.onClick();
              }}
              sx={{
                color: (t) => tone(t).contrastText,
                bgcolor: (t) => tone(t).main,
                "&:hover": { bgcolor: (t) => tone(t).dark },
                px: action.icon ? 0 : 1.25,
                width: action.icon ? 28 : "auto",
                // 28px button in the pill's 28px inner area (36px − 8px py) → a
                // uniform 4px inset, snug, no floating margin.
                height: 28,
                borderRadius: 999,
                fontSize: 12,
                fontWeight: 600,
              }}
            >
              {action.icon ?? action.label}
            </IconButton>
          )}
        </Stack>
      </Stack>
    </Box>
  );
}
