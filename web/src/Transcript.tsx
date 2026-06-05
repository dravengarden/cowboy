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

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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
import { CLAUDE_VERBS } from "./claudeVerbs";
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

// Claude Code's morphing star glyph — the literal frames its terminal spinner
// cycles through (from CC's Spinner/utils.ts `getDefaultCharacters`, the
// non-darwin set). The "·→✢→*→✶→✻→✽" growth IS the animation; CC advances one
// frame every 120ms (`Math.floor(time / 120)`), so we match that exactly. The
// 黑话 verb bank lives in ./claudeVerbs (full 187-word copy of CC's own list).
const CLAUDE_SPINNER_FRAMES = ["·", "✢", "*", "✶", "✻", "✽"];
const CLAUDE_FRAME_MS = 120;

// Claude-code indicator: a character-level recreation of Claude Code's own
// status line — its morphing star glyph (CLAUDE_SPINNER_FRAMES @ 120ms, in CC's
// terracotta brand fill #D97757) + a shimmer-swept verb that rotates through the
// playful 黑话 bank (~3.5s) and a literal "…". Under `prefers-reduced-motion` the
// glyph freezes on frame 0 ("·") and the verb shimmer collapses to a static
// muted word (CC freezes the glyph the same way — `reducedMotion ? 0 : …`).
// Verb rotation is content, not motion, so it stays.
function ClaudeThinking(): React.JSX.Element {
  const theme = useTheme();
  const muted = theme.palette.text.secondary;
  const accent = "#D97757";
  const reducedMotion = globalThis.matchMedia?.(
    "(prefers-reduced-motion: reduce)",
  ).matches;
  const [vi, setVi] = useState(0);
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    const id = globalThis.setInterval(() => setVi((v) => v + 1), 3500);
    return () => globalThis.clearInterval(id);
  }, []);
  useEffect(() => {
    if (reducedMotion) return undefined;
    const id = globalThis.setInterval(
      () => setFrame((f) => f + 1),
      CLAUDE_FRAME_MS,
    );
    return () => globalThis.clearInterval(id);
  }, [reducedMotion]);
  const verb = CLAUDE_VERBS[vi % CLAUDE_VERBS.length] ?? "Thinking";
  const glyph = reducedMotion
    ? CLAUDE_SPINNER_FRAMES[0]
    : CLAUDE_SPINNER_FRAMES[frame % CLAUDE_SPINNER_FRAMES.length];

  return (
    <Stack
      direction="row"
      spacing={0.75}
      alignItems="center"
      sx={{ py: 0.5, alignSelf: "flex-start" }}
    >
      <Box
        component="span"
        aria-hidden
        sx={{
          width: 14,
          textAlign: "center",
          fontSize: 14,
          lineHeight: 1,
          fontWeight: 700,
          color: accent,
          // Monospace + tabular so the glyph swap doesn't shift the verb.
          fontFamily:
            "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        }}
      >
        {glyph}
      </Box>
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

// Pixels above the bottom past which the scroll-to-latest FAB appears. Matches
// Slack / Telegram / Linear — roughly one short message worth of hidden content.
const FAB_THRESHOLD = 200;

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
        // Assistant replies stretch the full column so they align flush with the
        // full-width tool cards and use the whole screen width (no ragged column
        // of short, content-hugging bubbles). Only the user's own messages stay
        // as a right-aligned bubble, the conventional "my message" affordance.
        alignSelf: mine ? "flex-end" : "stretch",
        maxWidth: mine ? { xs: "88%", sm: "78%" } : "100%",
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

// `memo`'d: with `derive` memoized upstream, each item keeps a stable identity
// across renders that don't change the timeline (e.g. scroll-threshold or
// permission-sheet state). Without this, any such re-render would re-run the
// markdown parser + syntax highlighter for every visible row — the source of
// the scroll jitter on mobile.
const ItemView = memo(function ItemView({
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
            alignSelf: "stretch",
            maxWidth: "100%",
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
});

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
  // Memoized on `timeline` identity: `applyEnvelope` (store.ts) only hands us a
  // new array when a new event actually lands, so this O(n) fold runs once per
  // event — NOT on every scroll-driven re-render. Stable item identities also
  // let the `memo`'d `ItemView` rows skip re-rendering.
  const items = useMemo(() => derive(timeline), [timeline]);
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
  const dead = status === "exited" || status === "crashed";
  // "Working" is broader than the raw `busy` status: a tool call still in flight
  // means the agent is mid-work even when the session status momentarily reads
  // "running". The daemon flips status → running the instant a turn's prompt()
  // resolves, and reconnects re-broadcast status before replaying the timeline,
  // so a pending/in_progress tool can coexist with a non-busy status for a beat.
  // Keying the indicator off status alone made it blink out then (the reported
  // bug). `derive` settles tools to terminal on turn_end, so this can't latch on
  // a finished turn; the `dead` guard stops it latching on a crashed one.
  const toolInFlight =
    !dead &&
    items.some(
      (it) =>
        it.kind === "tool" &&
        (it.status === "pending" || it.status === "in_progress"),
    );
  const working = busy || toolInFlight;
  const lastIdx = items.length - 1;
  const lastItem = lastIdx >= 0 ? items[lastIdx] : undefined;
  // The last item is "streaming" if the agent is working AND it's an
  // assistant message or a thought (both grow chunk by chunk). Tool calls
  // have their own in_progress visual.
  const lastIsStreamingAssistant =
    working &&
    !!lastItem &&
    ((lastItem.kind === "message" && lastItem.role === "assistant") ||
      lastItem.kind === "thought");
  // Show the trailing "dots" row when working AND we're NOT already showing
  // a caret-tipped streaming assistant bubble at the bottom (i.e. between
  // sending a prompt and the first chunk landing, or after a tool call
  // completes while waiting for the model to start text again).
  const showTrailingDots = working && !lastIsStreamingAssistant;
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
    // Round to whole pixels: a raw fractional getBoundingClientRect height
    // re-measures to a hair-different value on each pass (80.33 → 80.34 …),
    // and each change nudges every following row's offset — visible as a
    // shimmy when scrolling up through measured rows.
    measureElement: (el) => Math.round(el.getBoundingClientRect().height),
  });

  // Whether the viewport sits far enough above the bottom to surface the
  // scroll-to-latest FAB (a 3-message transcript that fits the viewport never
  // does). Stored as a BOOLEAN, not the raw pixel distance: `onScroll` fires
  // every frame, and setting a numeric state each time would re-render the
  // whole Transcript on every scroll tick (the jitter). A boolean only flips
  // when crossing the threshold, so React bails out of the in-between renders.
  const [farFromBottom, setFarFromBottom] = useState(false);

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
      // Functional update returning the unchanged value → React skips the
      // re-render, so mid-scroll frames that don't cross the threshold are free.
      setFarFromBottom((prev) => (dist > FAB_THRESHOLD) === prev ? prev : dist > FAB_THRESHOLD);
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

  // Opening / switching a session always starts pinned to the latest message
  // (chat default). The component isn't remounted per session, so `stick`
  // would otherwise carry over a scrolled-up position from the previous
  // session. We also can't rely on a single scroll: react-virtual measures
  // rows lazily, so the first `scrollToIndex` lands short of the true bottom
  // (estimated 80px rows are usually shorter than real ones). Re-pin via
  // `scrollTop = scrollHeight` across a few frames so it converges to the
  // actual bottom as heights settle.
  useLayoutEffect(() => {
    stick.current = true;
    setDetached(false);
    setFarFromBottom(false);
    let raf = 0;
    let tries = 0;
    const pin = (): void => {
      const el = parentRef.current;
      if (el) el.scrollTop = el.scrollHeight;
      if (++tries < 5) raf = requestAnimationFrame(pin);
    };
    raf = requestAnimationFrame(pin);
    return () => cancelAnimationFrame(raf);
  }, [sessionId]);

  // Auto-snap only on rowCount changes, ONLY if we're still stuck.
  // We also recompute `distFromBottom` here because the virtualizer's
  // total height changes when new events stream in — the user may be
  // detached, watching a partial reply, and the FAB needs to (start to)
  // appear without an actual scroll event firing.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (el) {
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      setFarFromBottom(dist > FAB_THRESHOLD);
    }
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
            const streaming = working && vi.index === lastIdx;
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
      {detached && farFromBottom && (
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
