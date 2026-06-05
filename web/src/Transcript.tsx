// Virtualized transcript. One row per derived `RenderItem`; row heights are
// measured dynamically so streamed-in markdown / code blocks / images grow
// naturally without us pre-computing sizes.
//
// Why virtual at all: a long session (claude with hundreds of streamed
// chunks + tool cards) renders thousands of DOM nodes otherwise — on mobile
// that scrolls badly and locks up the main thread. `@tanstack/react-virtual`
// keeps DOM proportional to the viewport, with `measureElement` for
// variable heights.
//
// Why no virtual on tool/permission CARDS (those have their own collapse
// state): row-level virtualization unmounts and remounts rows as they leave
// the viewport. To preserve per-card UI state across scroll we'd need to
// hoist that state into a Map keyed by item id; v0 accepts that re-opening
// a card after scrolling away is required. Tool cards default to collapsed,
// so this is mostly a non-issue.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  Box,
  Button,
  Chip,
  CircularProgress,
  Fab,
  LinearProgress,
  Paper,
  Skeleton,
  Stack,
  Typography,
  keyframes,
  useTheme,
} from "@mui/material";
import {
  ArrowDownward,
  AutoAwesome,
  CheckCircle,
  Code,
  Construction,
  ErrorOutline,
  ExpandLess,
  ExpandMore,
  Folder,
  Psychology,
  RadioButtonUnchecked,
  Search,
  Terminal,
  WarningAmberRounded,
} from "@mui/icons-material";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Markdown } from "./Markdown";
import { derive, type ContentChunk, type RenderItem } from "./derive";
import type { Envelope, Status } from "./protocol";
import { send } from "./store";
import { BottomSheet } from "./_shell";

// --- Loading primitives -----------------------------------------------------

// Soft opacity breathing — used both for the in-flight tool card and the
// Claude "thinking" spark. CSS keyframes instead of a JS animation lib so it's
// free on bundle size and runs on the compositor (smooth on low-end phones).
const pulse = keyframes`
  0%, 100% { opacity: 1; }
  50%      { opacity: 0.55; }
`;

// Claude Code's "prompt keyword shimmer": a highlight band sweeps across the
// verb word (color applied via background-clip:text in sx).
const shimmer = keyframes`to { background-position: -200% 0; }`;

// Claude Code's signature playful "spinner verbs" (the 黑话 vocabulary) — a
// rotating bank of whimsical gerunds instead of a flat "Thinking…". This is
// the Claude-flavored loading personality; Codex deliberately stays plain.
const CLAUDE_VERBS = [
  "Thinking",
  "Cogitating",
  "Pondering",
  "Ruminating",
  "Percolating",
  "Noodling",
  "Befuddling",
  "Conjuring",
  "Simmering",
  "Marinating",
  "Frolicking",
  "Discombobulating",
  "Synthesizing",
  "Tinkering",
  "Brewing",
];

// Claude-code indicator: faithful to Claude Code's own status line — a pulsing
// terracotta spark (#D97757, its brand fill) + a shimmer-swept verb that rotates
// through the playful 黑话 bank (~3.5s) and a literal "…". The shimmer/pulse
// collapse to a static muted word under `prefers-reduced-motion` (Anthropic
// shipped a fix specifically because their shimmer ignored that setting — we
// honor it up front). Verb rotation is content, not motion, so it stays.
function ClaudeThinking(): React.JSX.Element {
  const theme = useTheme();
  const muted = theme.palette.text.secondary;
  const accent = "#D97757";
  const [vi, setVi] = useState(0);
  useEffect(() => {
    const id = globalThis.setInterval(() => setVi((v) => v + 1), 3500);
    return () => globalThis.clearInterval(id);
  }, []);
  const verb = CLAUDE_VERBS[vi % CLAUDE_VERBS.length] ?? "Thinking";

  return (
    <Stack
      direction="row"
      spacing={0.75}
      alignItems="center"
      sx={{ py: 0.5, alignSelf: "flex-start" }}
    >
      <AutoAwesome
        sx={{
          fontSize: 14,
          color: accent,
          animation: `${pulse} 1.6s ease-in-out infinite`,
          "@media (prefers-reduced-motion: reduce)": { animation: "none" },
        }}
      />
      <Typography
        variant="caption"
        sx={{
          fontWeight: 500,
          letterSpacing: 0.2,
          background: `linear-gradient(90deg, ${muted} 0%, ${muted} 40%, ${accent} 50%, ${muted} 60%, ${muted} 100%)`,
          backgroundSize: "200% 100%",
          WebkitBackgroundClip: "text",
          backgroundClip: "text",
          color: "transparent",
          animation: `${shimmer} 2.4s linear infinite`,
          "@media (prefers-reduced-motion: reduce)": {
            animation: "none",
            background: "none",
            color: muted,
            WebkitTextFillColor: muted,
          },
        }}
      >
        {verb}…
      </Typography>
    </Stack>
  );
}

// Default indicator (Codex + any non-Claude provider): the plain Material UI
// loading look — a small CircularProgress + a muted "Thinking…" caption. No
// shimmer, no jargon; MUI's own spinner carries it, deliberately understated
// next to the Claude flavor.
function DefaultThinking(): React.JSX.Element {
  return (
    <Stack
      direction="row"
      spacing={1}
      alignItems="center"
      sx={{ py: 0.5, alignSelf: "flex-start", color: "text.secondary" }}
    >
      <CircularProgress size={16} thickness={5} />
      <Typography variant="caption" color="text.secondary">
        Thinking…
      </Typography>
    </Stack>
  );
}

// The trailing "agent is working" indicator. Claude-code gets its playful
// shimmer-jargon personality; every other provider gets the default MUI spinner.
function ThinkingIndicator({
  provider,
}: {
  provider: string;
}): React.JSX.Element {
  return provider === "claude-code" ? <ClaudeThinking /> : <DefaultThinking />;
}

// Blinking text caret — Claude.ai / Cursor put one at the end of streaming
// text so the user knows the model is still producing. `steps(2)` makes it
// blink-on / blink-off rather than fading, which reads as more "alive".
const blink = keyframes`
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
`;

function StreamingCaret(): React.JSX.Element {
  return (
    <Box
      component="span"
      aria-hidden
      sx={{
        display: "inline-block",
        width: "0.55em",
        height: "1em",
        ml: 0.25,
        verticalAlign: "text-bottom",
        bgcolor: "text.primary",
        animation: `${blink} 1s steps(2, jump-none) infinite`,
      }}
    />
  );
}

function toolColor(status: string): "default" | "success" | "error" | "warning" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "in_progress") return "warning";
  return "default";
}

function toolIcon(kind: string): React.ReactElement {
  switch (kind) {
    case "read":
      return <Folder fontSize="medium" />;
    case "edit":
      return <Code fontSize="medium" />;
    case "execute":
      return <Terminal fontSize="medium" />;
    case "search":
      return <Search fontSize="medium" />;
    default:
      return <Construction fontSize="medium" />;
  }
}

function ChunkView({
  chunk,
  invert,
}: {
  chunk: ContentChunk;
  invert: boolean;
}): React.JSX.Element {
  if (chunk.type === "image") {
    return (
      <Box
        component="img"
        src={chunk.src}
        alt={chunk.alt ?? ""}
        sx={{
          maxWidth: "100%",
          // Cap image preview height in dvh so it scales with viewport
          // (mobile portrait stays bounded; desktop can show bigger).
          maxHeight: "50dvh",
          display: "block",
          borderRadius: 1,
          my: 0.5,
        }}
        loading="lazy"
      />
    );
  }
  return <Markdown text={chunk.text} invert={invert} />;
}

function MessageBubble({
  role,
  chunks,
  streaming,
}: {
  role: "assistant" | "user";
  chunks: ContentChunk[];
  /** When true, append a blinking caret after the last text chunk to signal
   *  the model is still producing. */
  streaming?: boolean;
}): React.JSX.Element {
  const mine = role === "user";
  const lastChunkIdx = chunks.length - 1;
  return (
    <Paper
      variant="outlined"
      sx={{
        p: { xs: 1, sm: 1.25 },
        alignSelf: mine ? "flex-end" : "flex-start",
        maxWidth: { xs: "96%", sm: "92%" },
        bgcolor: mine ? "primary.main" : "background.paper",
        color: mine ? "primary.contrastText" : "text.primary",
        overflow: "hidden",
      }}
    >
      {chunks.map((c, i) => (
        <Box key={i} sx={{ position: "relative" }}>
          <ChunkView chunk={c} invert={mine} />
          {streaming && i === lastChunkIdx && c.type === "text" && (
            <StreamingCaret />
          )}
        </Box>
      ))}
    </Paper>
  );
}

function ToolCard({
  item,
}: {
  item: Extract<RenderItem, { kind: "tool" }>;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const hasDetail = item.rawInput !== undefined || item.content !== undefined;
  const running = item.status === "in_progress" || item.status === "pending";
  return (
    <Paper
      variant="outlined"
      sx={{
        alignSelf: "stretch",
        overflow: "hidden",
        // Subtle breathing while a tool is mid-flight; nothing while
        // completed/failed (those are static states).
        animation: running ? `${pulse} 1.6s ease-in-out infinite` : undefined,
      }}
    >
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        sx={{
          p: 1,
          cursor: hasDetail ? "pointer" : "default",
          "&:hover": hasDetail ? { bgcolor: "action.hover" } : undefined,
        }}
        onClick={(): void => {
          if (hasDetail) setOpen((o) => !o);
        }}
      >
        {toolIcon(item.toolKind)}
        <Typography
          variant="body2"
          sx={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            fontFamily:
              item.toolKind === "execute"
                ? "ui-monospace, SFMono-Regular, Menlo, monospace"
                : undefined,
          }}
        >
          {item.title}
        </Typography>
        <Chip size="small" color={toolColor(item.status)} label={item.status} variant="outlined" />
        {hasDetail && (open ? <ExpandLess fontSize="medium" /> : <ExpandMore fontSize="medium" />)}
      </Stack>
      {open && hasDetail && (
        <Box sx={{ borderTop: 1, borderColor: "divider", p: 1, bgcolor: "action.hover" }}>
          {item.rawInput !== undefined && (
            <>
              <Typography variant="caption" color="text.secondary">
                Input
              </Typography>
              <Markdown
                text={"```json\n" + JSON.stringify(item.rawInput, null, 2) + "\n```"}
              />
            </>
          )}
          {item.content !== undefined ? (
            <>
              <Typography variant="caption" color="text.secondary">
                Output
              </Typography>
              <Markdown
                text={
                  "```json\n" + JSON.stringify(item.content, null, 2) + "\n```"
                }
              />
            </>
          ) : (
            running && (
              <>
                <Typography variant="caption" color="text.secondary">
                  Output
                </Typography>
                <Skeleton animation="wave" width="80%" />
                <Skeleton animation="wave" width="60%" />
                <Skeleton animation="wave" width="40%" />
              </>
            )
          )}
        </Box>
      )}
    </Paper>
  );
}

// The agent's plan rendered as a task-completion checklist (ACP `plan` update).
// Beyond the per-entry check, it carries a progress summary — a "done/total"
// counter and a determinate bar — so the turn's completion is legible at a
// glance without reading every line. Three entry states are distinguished:
// completed (green check, struck + muted), in_progress (amber spinner, bold),
// pending (empty circle). When every entry is done the bar + header flip to
// success — the closest cowboy gets to a "task complete" marker.
function PlanCard({
  item,
}: {
  item: Extract<RenderItem, { kind: "plan" }>;
}): React.JSX.Element {
  const total = item.entries.length;
  const done = item.entries.filter((e) => e.status === "completed").length;
  const allDone = total > 0 && done === total;
  const pct = total > 0 ? (done / total) * 100 : 0;
  return (
    <Paper variant="outlined" sx={{ p: 1.25, alignSelf: "stretch" }}>
      <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 0.5 }}>
        <Typography variant="overline" sx={{ lineHeight: 1.4 }}>
          Plan
        </Typography>
        <Box sx={{ flex: 1 }} />
        {allDone && <CheckCircle fontSize="small" color="success" />}
        <Typography
          variant="caption"
          color={allDone ? "success.main" : "text.secondary"}
          sx={{ fontWeight: 600, fontVariantNumeric: "tabular-nums" }}
        >
          {done}/{total}
        </Typography>
      </Stack>
      {total > 0 && (
        <LinearProgress
          variant="determinate"
          value={pct}
          color={allDone ? "success" : "primary"}
          sx={{ height: 6, borderRadius: 3, mb: 1 }}
        />
      )}
      <Stack spacing={0.5}>
        {item.entries.map((e, j) => {
          const completed = e.status === "completed";
          const inProgress = e.status === "in_progress";
          return (
            <Stack key={j} direction="row" spacing={1} alignItems="center">
              {completed ? (
                <CheckCircle fontSize="medium" color="success" />
              ) : inProgress ? (
                <CircularProgress size={16} thickness={5} color="warning" sx={{ mx: 0.25 }} />
              ) : (
                <RadioButtonUnchecked fontSize="medium" color="disabled" />
              )}
              <Typography
                variant="body2"
                color={completed ? "text.disabled" : "text.primary"}
                sx={{
                  fontWeight: inProgress ? 600 : 400,
                  textDecoration: completed ? "line-through" : "none",
                }}
              >
                {e.content}
              </Typography>
            </Stack>
          );
        })}
      </Stack>
    </Paper>
  );
}

function PermissionCard({
  item,
}: {
  item: Extract<RenderItem, { kind: "permission" }>;
}): React.JSX.Element {
  // Two visual states, both compact in-timeline markers. The actual decision
  // happens in the dedicated PermissionSheet (a modal that pops at the
  // Transcript root), not inline: a tool-approval is high-stakes and easy to
  // scroll past / mis-tap as inline buttons on a phone, so it earns a focused
  // dialog. The timeline keeps only a record:
  //   Pending  = an amber "permission requested" line (the sheet is open / can
  //              be reopened via the sticky Review control).
  //   Resolved = a subtle italic one-line summary of what was decided (a
  //              glaring orange card would otherwise sit in the log forever).
  if (item.resolved) {
    const rejected = item.chosen?.toLowerCase().startsWith("reject") ?? false;
    return (
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        sx={{ alignSelf: "flex-start", color: "text.secondary", px: 0.5 }}
      >
        <Typography variant="caption" sx={{ fontStyle: "italic" }}>
          {rejected ? "Rejected" : "Allowed"}
          {item.chosen ? `: ${item.chosen}` : ""} · {item.title}
        </Typography>
      </Stack>
    );
  }
  return (
    <Stack
      direction="row"
      spacing={1}
      alignItems="center"
      sx={{ alignSelf: "flex-start", color: "warning.main", px: 0.5 }}
    >
      <WarningAmberRounded fontSize="medium" sx={{ flexShrink: 0 }} />
      <Typography variant="caption" sx={{ fontWeight: 600, minWidth: 0 }}>
        Permission requested · {item.title}
      </Typography>
    </Stack>
  );
}

// The dedicated approval dialog. Mobile → the shared DetentSheet (sheet on the
// keyboard-safe bottom); desktop → a centered Dialog. The ACP options become
// full-width, ≥48px stacked buttons in the sheet footer — a forced, unmissable
// decision instead of small inline chips (ui.md §7). Dismissing the sheet does
// NOT resolve the request (no accidental reject); the Transcript shows a sticky
// "Review" control to reopen it, and the agent stays blocked until a real pick.
function PermissionSheet({
  sessionId,
  item,
  open,
  onClose,
}: {
  sessionId: string;
  item: Extract<RenderItem, { kind: "permission" }>;
  open: boolean;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <BottomSheet
      open={open}
      onClose={onClose}
      title={
        <Stack direction="row" spacing={1} alignItems="center">
          <WarningAmberRounded color="warning" />
          <span>Permission required</span>
        </Stack>
      }
      actions={
        <Stack spacing={1} sx={{ width: "100%" }}>
          {item.options.map((opt) => (
            <Button
              key={opt.optionId}
              fullWidth
              variant={opt.kind.startsWith("reject") ? "outlined" : "contained"}
              color={opt.kind.startsWith("reject") ? "error" : "primary"}
              onClick={(): void =>
                send({
                  type: "permission",
                  session_id: sessionId,
                  request_id: item.requestId,
                  option_id: opt.optionId,
                })
              }
              // Full-width, ≥48px stacked rows: the highest-stakes tap in the
              // app deserves the most reachable target on touch (ui.md §7).
              sx={{ minHeight: { xs: 48, sm: 44 }, fontSize: { xs: 16, sm: 15 } }}
            >
              {opt.name}
            </Button>
          ))}
        </Stack>
      }
    >
      <Typography variant="body2" sx={{ wordBreak: "break-word" }}>
        {item.title}
      </Typography>
    </BottomSheet>
  );
}

function ItemView({
  item,
  streaming,
}: {
  item: RenderItem;
  /** True when this item is the last assistant chunk-bearing item and the
   *  session is still busy. Adds a blinking caret / dots accordingly. */
  streaming?: boolean;
}): React.JSX.Element | null {
  switch (item.kind) {
    case "message":
      return (
        <MessageBubble
          role={item.role}
          chunks={item.chunks}
          streaming={!!streaming && item.role === "assistant"}
        />
      );
    case "thought":
      return (
        <Stack
          direction="row"
          spacing={1}
          sx={{
            color: "text.secondary",
            alignSelf: "flex-start",
            maxWidth: { xs: "96%", sm: "92%" },
          }}
        >
          <Psychology fontSize="medium" />
          <Box sx={{ fontStyle: "italic", fontSize: "0.875rem", flex: 1 }}>
            {/* `derive` drops empty thoughts, so a thought item always carries
                text here — no perpetual-spinner fallback. */}
            <Markdown text={item.text} />
            {streaming && <StreamingCaret />}
          </Box>
        </Stack>
      );
    case "tool":
      return <ToolCard item={item} />;
    case "plan":
      return <PlanCard item={item} />;
    case "permission":
      return <PermissionCard item={item} />;
    case "lifecycle":
      return (
        <Stack
          direction="row"
          spacing={1}
          alignItems="center"
          sx={{ color: item.status === "crashed" ? "error.main" : "text.secondary" }}
        >
          <ErrorOutline fontSize="medium" />
          <Typography variant="caption">
            {item.status}
            {item.detail ? `: ${item.detail}` : ""}
          </Typography>
        </Stack>
      );
  }
}

export function Transcript({
  sessionId,
  timeline,
  status,
  provider,
}: {
  sessionId: string;
  timeline: Envelope[];
  status: Status;
  /** Drives the per-provider "thinking" indicator flavor (color + verbs). */
  provider: string;
}): React.JSX.Element {
  const items = derive(timeline);
  // Latest unresolved tool-permission request, if any → drives the dedicated
  // PermissionSheet. `reviewClosedFor` remembers a request the user flicked the
  // sheet shut on, so it doesn't keep re-popping while still leaving the sticky
  // "Review" control to reopen it. A new request (different id) auto-opens.
  let pendingPermission: Extract<RenderItem, { kind: "permission" }> | undefined;
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it && it.kind === "permission" && !it.resolved) {
      pendingPermission = it;
      break;
    }
  }
  const pendingReqId = pendingPermission?.requestId ?? null;
  const [reviewClosedFor, setReviewClosedFor] = useState<string | null>(null);
  const permissionOpen =
    pendingPermission !== undefined && pendingReqId !== reviewClosedFor;
  const busy = status === "busy";
  const lastIdx = items.length - 1;
  const lastItem = lastIdx >= 0 ? items[lastIdx] : undefined;
  // The last item is "streaming" if the session is busy AND it's an
  // assistant message or a thought (both grow chunk by chunk). Tool calls
  // have their own in_progress visual.
  const lastIsStreamingAssistant =
    busy &&
    !!lastItem &&
    ((lastItem.kind === "message" && lastItem.role === "assistant") ||
      lastItem.kind === "thought");
  // Show the trailing "dots" row when busy AND we're NOT already showing
  // a caret-tipped streaming assistant bubble at the bottom (i.e. between
  // sending a prompt and the first chunk landing, or after a tool call
  // completes while waiting for the model to start text again).
  const showTrailingDots = busy && !lastIsStreamingAssistant;
  // The virtualizer's row count includes a phantom "dots" row at the end
  // when we're showing it. Keeping it as a virtualized row (rather than a
  // sibling) means scroll-stickiness Just Works.
  const rowCount = items.length + (showTrailingDots ? 1 : 0);
  const parentRef = useRef<HTMLDivElement>(null);
  // "stick-to-bottom" UX, done properly this time:
  //
  // Previous bug: we listened to `onScroll` to decide if the user "wanted"
  // the bottom; but `onScroll` ALSO fires for our own `scrollToIndex`. So
  // every streamed chunk: snap-to-bottom → onScroll → atBottom=true →
  // stick=true → user wheels up → next chunk → snap-back. Unkillable.
  //
  // New model:
  // - Treat user intent as a separate signal: a wheel / touchstart / arrow
  //   key on the container means "I am leaving the bottom" — set stick=false
  //   *immediately*, before any subsequent programmatic scroll can confuse
  //   us.
  // - scroll-event only re-enables stick when the resulting position is
  //   actually at the bottom (the user manually scrolled all the way back).
  // - Auto-snap on new items uses the **`stick` value at the time of the
  //   commit** — i.e. via a ref so we don't re-render to keep it.
  // - A floating "↓" button surfaces when the user is detached so they can
  //   re-stick without scrolling manually.
  const stick = useRef(true);
  const [detached, setDetached] = useState(false);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 80,
    overscan: 6,
    measureElement: (el) => el.getBoundingClientRect().height,
  });

  // How far above the bottom the viewport currently sits. The scroll-to-
  // latest FAB only renders when this exceeds a small threshold, so a 3-
  // message transcript that already fits the viewport doesn't ever surface
  // a useless button.
  const [distFromBottom, setDistFromBottom] = useState(0);

  // Wire user-intent listeners ONCE; they read parentRef each time so the
  // dependency on the ref's contents stays out of React's eyes.
  useEffect(() => {
    const el = parentRef.current;
    if (!el) return undefined;
    const detach = (): void => {
      if (stick.current) {
        stick.current = false;
        setDetached(true);
      }
    };
    const onScroll = (): void => {
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      setDistFromBottom(dist);
      const atBottom = dist < 24;
      if (atBottom && !stick.current) {
        stick.current = true;
        setDetached(false);
      }
    };
    el.addEventListener("wheel", detach, { passive: true });
    el.addEventListener("touchstart", detach, { passive: true });
    el.addEventListener("scroll", onScroll, { passive: true });
    // Keyboard scrolls (PgUp / arrows) — listen on the container so it must
    // be focused first; that's fine, hits the rare desktop case.
    el.addEventListener("keydown", detach);
    return () => {
      el.removeEventListener("wheel", detach);
      el.removeEventListener("touchstart", detach);
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("keydown", detach);
    };
  }, []);

  // Auto-snap only on rowCount changes, ONLY if we're still stuck.
  // We also recompute `distFromBottom` here because the virtualizer's
  // total height changes when new events stream in — the user may be
  // detached, watching a partial reply, and the FAB needs to (start to)
  // appear without an actual scroll event firing.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (el) setDistFromBottom(el.scrollHeight - el.scrollTop - el.clientHeight);
    if (!stick.current || rowCount === 0) return;
    virtualizer.scrollToIndex(rowCount - 1, { align: "end" });
  }, [rowCount, virtualizer]);

  function jumpToBottom(): void {
    stick.current = true;
    setDetached(false);
    if (rowCount > 0) {
      virtualizer.scrollToIndex(rowCount - 1, { align: "end" });
    }
  }

  return (
    <Box sx={{ flex: 1, position: "relative", overflow: "hidden" }}>
      <Box
        ref={parentRef}
        tabIndex={0}
        sx={{
          height: "100%",
          overflowY: "auto",
          overflowX: "hidden",
          px: { xs: 1, sm: 2 },
          py: { xs: 1, sm: 1.5 },
          contain: "strict",
          // Hide focus ring; we keep tabIndex for keyboard scroll capture.
          outline: "none",
          overscrollBehavior: "contain",
        }}
      >
        <Box
          sx={{
            height: virtualizer.getTotalSize(),
            width: "100%",
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((vi) => {
            const isTrailingDots = showTrailingDots && vi.index === items.length;
            const item = isTrailingDots ? undefined : items[vi.index];
            const streaming = busy && vi.index === lastIdx;
            return (
              <Box
                key={vi.key}
                data-index={vi.index}
                ref={virtualizer.measureElement}
                sx={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${vi.start}px)`,
                  py: 0.625,
                  display: "flex",
                  flexDirection: "column",
                }}
              >
                {isTrailingDots ? (
                  <ThinkingIndicator provider={provider} />
                ) : item ? (
                  <ItemView item={item} streaming={streaming} />
                ) : null}
              </Box>
            );
          })}
        </Box>
      </Box>
      {detached && distFromBottom > 200 && (
        // Threshold matches Slack / Telegram / Linear: only surface the
        // jump-down affordance when there's at least ~one short message
        // worth of hidden content below. A three-line transcript that
        // fits the viewport never sees this button.
        <Fab
          size="small"
          color="primary"
          aria-label="scroll to latest"
          onClick={jumpToBottom}
          sx={{
            position: "absolute",
            bottom: 16,
            // The right pane spans the full device width on mobile (no
            // sidebar), so in landscape this would sit under the notch /
            // rounded corner. Floor the inset to keep it clear (0 off-device,
            // so desktop is unchanged) — ui.md §7.
            right: "max(env(safe-area-inset-right), 16px)",
            zIndex: 1,
            boxShadow: 3,
          }}
        >
          <ArrowDownward />
        </Fab>
      )}
      {pendingPermission && (
        <PermissionSheet
          sessionId={sessionId}
          item={pendingPermission}
          open={permissionOpen}
          onClose={(): void => setReviewClosedFor(pendingReqId)}
        />
      )}
      {pendingPermission && !permissionOpen && (
        // The sheet was flicked away but the request is still unresolved — keep
        // a prominent, centered reopen affordance so a required decision is
        // never stranded. Centered (not the bottom-right jump Fab's corner) and
        // amber to read as "action needed", clear of the landscape edge.
        <Fab
          variant="extended"
          color="warning"
          aria-label="review permission request"
          onClick={(): void => setReviewClosedFor(null)}
          sx={{
            position: "absolute",
            bottom: 16,
            left: "50%",
            transform: "translateX(-50%)",
            zIndex: 2,
            boxShadow: 3,
          }}
        >
          <WarningAmberRounded sx={{ mr: 1 }} />
          Review permission
        </Fab>
      )}
    </Box>
  );
}
