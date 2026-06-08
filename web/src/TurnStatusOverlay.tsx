import { useEffect, useRef, useState } from "react";
import { Box, IconButton, Stack, Typography, alpha } from "@mui/material";
import type { PaletteColor, Theme } from "@mui/material";
import ChatBubbleOutlineRounded from "@mui/icons-material/ChatBubbleOutlineRounded";
import CheckCircleRounded from "@mui/icons-material/CheckCircleRounded";
import ReplayRounded from "@mui/icons-material/ReplayRounded";
import ErrorOutlineRounded from "@mui/icons-material/ErrorOutlineRounded";
import KeyOffRounded from "@mui/icons-material/KeyOffRounded";
import ArrowUpwardRounded from "@mui/icons-material/ArrowUpwardRounded";
import CloseRounded from "@mui/icons-material/CloseRounded";
import ExpandMoreRounded from "@mui/icons-material/ExpandMoreRounded";
import type { Status } from "./protocol";
import {
  dismissAwaiting,
  requestSendQueued,
  resumeTurn,
  retryTurn,
  useJudgeResult,
} from "./store";

type Kind = "awaiting" | "done" | "interrupted" | "error" | "no-key";
type PaletteKey = "primary" | "success" | "warning" | "error" | "info";

// The unified "turn status" overlay (replaces the old AwaitingBar): one floating
// frosted pill above the composer that surfaces a NON-RUNNING turn-end state and
// its actions. Hidden while the agent is working (busy) or on a fresh session —
// it only appears when something needs your attention. Colour-coded per the locked
// palette: awaiting=purple, done=green, interrupted=amber, error=red, no-key=blue.
//
// `awaiting`/`done` come from the confirm-detect judge; `interrupted`/`error` from
// the session status; `no-key` when the judge can't run and a queue is held.

function deriveKind(args: {
  status: Status;
  awaitingUser: boolean;
  done: boolean;
  queueLen: number;
  hasKey: boolean;
}): Kind | null {
  const { status, awaitingUser, done, queueLen, hasKey } = args;
  if (status === "busy" || status === "starting") return null; // working / fresh
  if (status === "crashed") return "error";
  if (status === "interrupted") return "interrupted";
  if (awaitingUser) return "awaiting";
  if (done) return "done";
  if (!hasKey && queueLen > 0) return "no-key"; // queue held, can't judge
  return null;
}

const KIND_META: Record<Kind, { color: PaletteKey; Icon: typeof ChatBubbleOutlineRounded; label: string }> = {
  awaiting: { color: "primary", Icon: ChatBubbleOutlineRounded, label: "Waiting for your reply" },
  done: { color: "success", Icon: CheckCircleRounded, label: "Task complete" },
  interrupted: { color: "warning", Icon: ReplayRounded, label: "Turn interrupted" },
  error: { color: "error", Icon: ErrorOutlineRounded, label: "Agent error" },
  "no-key": { color: "info", Icon: KeyOffRounded, label: "Queue held · no judge key" },
};

export function TurnStatusOverlay({
  sessionId,
  status,
  awaitingUser,
  done,
  queue,
  hasKey,
  onFocusComposer,
  onConfigure,
}: {
  sessionId: string;
  status: Status;
  awaitingUser: boolean;
  done: boolean;
  queue: { id: string }[];
  hasKey: boolean;
  onFocusComposer: () => void;
  onConfigure: () => void;
}): React.JSX.Element | null {
  const kind = deriveKind({ status, awaitingUser, done, queueLen: queue.length, hasKey });
  const judge = useJudgeResult(sessionId);
  const [hidden, setHidden] = useState(false);
  const [expanded, setExpanded] = useState(false);

  // A new state always re-shows (reset the local dismiss); collapse the detail.
  useEffect(() => {
    setHidden(false);
    setExpanded(false);
  }, [kind]);
  // The green "done" pill is just a confirmation — auto-fade after a few seconds.
  useEffect(() => {
    if (kind !== "done") return undefined;
    const t = setTimeout(() => setHidden(true), 4000);
    return () => clearTimeout(t);
  }, [kind]);

  // Publish the pill height so the transcript reserves it (sticky-not-covering).
  const measureRef = useRef<HTMLDivElement | null>(null);
  const visible = kind !== null && !hidden;
  useEffect(() => {
    const el = measureRef.current;
    if (!el || !visible) {
      document.documentElement.style.setProperty("--awaiting-h", "0px");
      return undefined;
    }
    const set = (): void => document.documentElement.style.setProperty("--awaiting-h", `${el.offsetHeight}px`);
    set();
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => {
      ro.disconnect();
      document.documentElement.style.setProperty("--awaiting-h", "0px");
    };
  }, [visible, expanded]);

  if (kind === null || hidden) return null;
  const meta = KIND_META[kind];
  const held = queue.length > 0;
  const tone = (t: Theme): PaletteColor => t.palette[meta.color];
  const showExpand = (kind === "awaiting" || kind === "done") && judge !== undefined;

  // The primary trailing action (besides the × dismiss), per kind.
  let action: { label: string; icon?: React.JSX.Element; onClick: () => void } | undefined;
  if (kind === "awaiting" && held) {
    action = { label: "Send", icon: <ArrowUpwardRounded sx={{ fontSize: 18 }} />, onClick: () => dismissAwaiting(sessionId) };
  } else if (kind === "interrupted") {
    action = { label: "Resume", onClick: () => resumeTurn(sessionId) };
  } else if (kind === "error") {
    action = { label: "Retry", onClick: () => retryTurn(sessionId) };
  } else if (kind === "no-key") {
    action = held
      ? { label: "Send", onClick: () => requestSendQueued(sessionId, queue[0]?.id ?? "") }
      : { label: "Set key", onClick: onConfigure };
  }

  const dismiss = (): void => {
    if (kind === "awaiting") dismissAwaiting(sessionId);
    else setHidden(true);
  };

  return (
    <Box
      ref={measureRef}
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
      <Stack alignItems="center" spacing={0.75} sx={{ maxWidth: "100%", pointerEvents: "auto" }}>
        {expanded && judge && (
          <Box
            sx={{
              width: "100%",
              maxWidth: 460,
              p: 1.25,
              borderRadius: 2,
              backgroundColor: (t) => alpha(t.palette.background.default, t.palette.mode === "dark" ? 0.86 : 0.92),
              backdropFilter: "blur(30px) saturate(200%)",
              WebkitBackdropFilter: "blur(30px) saturate(200%)",
              border: (t) => `1px solid ${alpha(tone(t).main, 0.3)}`,
              boxShadow: (t) => `0 6px 20px ${alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.45 : 0.18)}`,
              fontSize: 11,
              maxHeight: 280,
              overflow: "auto",
            }}
          >
            <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>
              {judge.layer} · {judge.model || "no model"} · {judge.latency_ms}ms · cache hit {judge.cache_hit}/miss{" "}
              {judge.cache_miss} · conf {judge.confidence.toFixed(2)}
            </Typography>
            <Typography variant="caption" sx={{ fontWeight: 600 }}>Input</Typography>
            <Box component="pre" sx={{ m: 0, mb: 0.75, whiteSpace: "pre-wrap", wordBreak: "break-word", fontSize: 11 }}>
              {judge.input || "—"}
            </Box>
            <Typography variant="caption" sx={{ fontWeight: 600 }}>Output</Typography>
            <Box component="pre" sx={{ m: 0, whiteSpace: "pre-wrap", wordBreak: "break-word", fontSize: 11 }}>
              {judge.output || "—"}
            </Box>
          </Box>
        )}
        <Stack
          role="status"
          direction="row"
          alignItems="center"
          spacing={1}
          onClick={onFocusComposer}
          sx={{
            cursor: "text",
            maxWidth: "100%",
            pl: 1.5,
            pr: 0.5,
            py: 0.5,
            borderRadius: 999,
            // Milky, mostly-opaque base + a thin tone tint so it's legible over the
            // busy transcript on desktop AND iOS (low-alpha-only bleeds text through).
            backgroundColor: (t) => alpha(t.palette.background.default, t.palette.mode === "dark" ? 0.82 : 0.88),
            backgroundImage: (t) => `linear-gradient(0deg, ${alpha(tone(t).main, 0.16)}, ${alpha(tone(t).main, 0.16)})`,
            backdropFilter: "blur(30px) saturate(200%)",
            WebkitBackdropFilter: "blur(30px) saturate(200%)",
            border: (t) => `1px solid ${alpha(tone(t).main, 0.32)}`,
            boxShadow: (t) => `0 6px 20px ${alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.45 : 0.18)}`,
          }}
        >
          <meta.Icon sx={{ fontSize: 18, color: (t) => tone(t).main }} />
          <Typography variant="body2" sx={{ fontWeight: 600, color: (t) => tone(t).main, whiteSpace: "nowrap" }}>
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
              <ExpandMoreRounded sx={{ fontSize: 18, transform: expanded ? "rotate(180deg)" : "none", transition: "transform .15s" }} />
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
                px: action.icon ? 0 : 1,
                width: action.icon ? 28 : "auto",
                height: 28,
                borderRadius: 999,
                fontSize: 12,
                fontWeight: 600,
              }}
            >
              {action.icon ?? action.label}
            </IconButton>
          )}
          <IconButton
            size="small"
            aria-label="Dismiss"
            onClick={(e): void => {
              e.stopPropagation();
              dismiss();
            }}
            sx={{ color: "text.secondary", width: 26, height: 26 }}
          >
            <CloseRounded sx={{ fontSize: 18 }} />
          </IconButton>
        </Stack>
      </Stack>
    </Box>
  );
}
