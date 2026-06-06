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
import { useReadingSettings } from "./readingSettings";
import {
  resetSticky,
  setSticky,
  useScrollNonce,
} from "./stickyStore";
import { BottomSheet, ImageLightbox } from "./_shell";

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
// non-darwin set). The "·→✢→*→✶→✻→✽" growth IS the animation. CC advances one
// frame every 120ms in the terminal, where the glyph is small. Rendered larger
// and higher-contrast on this web surface, that 120ms beat reads as jittery
// "蹦", so we deliberately diverge from CC and slow it to 200ms — a calmer
// 1.2s/cycle breath that still reads as alive. The 黑话 verb bank lives in
// ./claudeVerbs (full 187-word copy of CC's own list).
const CLAUDE_SPINNER_FRAMES = ["·", "✢", "*", "✶", "✻", "✽"];
const CLAUDE_FRAME_MS = 200;

// Claude-code indicator: a character-level recreation of Claude Code's own
// status line — its morphing star glyph (CLAUDE_SPINNER_FRAMES @ 200ms, in CC's
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

// A transcript image: a bounded, tappable thumbnail that opens the shared
// zoom/pan lightbox. The chunk's src is already a downscaled (~1568px) data URL
// (see attachments.ts), so it's light enough to inline here and sharp enough to
// zoom. plate={false}: chat images are screenshots/photos, not white-bg figures
// that need a frame.
function TranscriptImage({ src, alt }: { src: string; alt: string }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Box
        component="img"
        src={src}
        alt={alt}
        loading="lazy"
        onClick={(): void => setOpen(true)}
        sx={{
          maxWidth: "min(240px, 100%)",
          maxHeight: 240,
          objectFit: "cover",
          display: "block",
          borderRadius: 1,
          my: 0.5,
          cursor: "zoom-in",
        }}
      />
      <ImageLightbox
        images={[{ src, alt }]}
        index={open ? 0 : null}
        onIndex={(): void => undefined}
        onClose={(): void => setOpen(false)}
        plate={false}
      />
    </>
  );
}

function ChunkView({
  chunk,
  invert,
}: {
  chunk: ContentChunk;
  invert: boolean;
}): React.JSX.Element {
  if (chunk.type === "image") {
    return <TranscriptImage src={chunk.src} alt={chunk.alt ?? ""} />;
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
  const body = chunks.map((c, i) => (
    <Box key={i} sx={{ position: "relative" }}>
      <ChunkView chunk={c} invert={mine} />
      {streaming && i === lastChunkIdx && c.type === "text" && (
        <StreamingCaret />
      )}
    </Box>
  ));
  // Assistant replies render flush in the page (Zed-style): no border, no card
  // background, just markdown flowing inline. The per-item `py` in the
  // virtualizer row is the only separator between consecutive replies. Only the
  // user's own messages keep the right-aligned bubble — the conventional "my
  // message" affordance.
  if (!mine) {
    return (
      <Box sx={{ alignSelf: "stretch", maxWidth: "100%", color: "text.primary" }}>
        {body}
      </Box>
    );
  }
  return (
    <Paper
      variant="outlined"
      sx={{
        p: { xs: 1, sm: 1.25 },
        alignSelf: "flex-end",
        maxWidth: { xs: "88%", sm: "78%" },
        bgcolor: "primary.main",
        color: "primary.contrastText",
        overflow: "hidden",
      }}
    >
      {body}
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
            // Follow the reading-font setting like the prose does, instead of
            // letting MUI Typography pin the theme font — otherwise picking a
            // serif/sans reading face restyles the messages but the tool cards
            // stay on the system font, which reads as inconsistent. Shell
            // commands (execute) stay monospace: they're code, and a path full
            // of slashes in a serif is worse, not better.
            fontFamily:
              item.toolKind === "execute"
                ? "ui-monospace, SFMono-Regular, Menlo, monospace"
                : "var(--cowboy-reading-font, inherit)",
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
  // Reader-comfort controls (Settings → Reading). `fontScale` is applied as an
  // `em` on the scroll container so the markdown body + its em-relative
  // headings/code scale together while the MUI chrome (tool cards, captions)
  // keeps its fixed rem size; `padding` is the column's side gutter;
  // The reading font-family swaps via the `--cowboy-reading-font` CSS var that
  // useReadingFontFaces (mounted at the app root) sets + lazy-loads; code fences
  // keep their own monospace. A change re-renders here, and the virtualizer
  // re-measures row heights on the next pass, so live adjustments reflow
  // without a reload.
  const { fontScale, padding } = useReadingSettings();
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
  // - The on/off state is mirrored to the per-session stickyStore so the
  //   composer's sticky toggle reflects + drives it (it shows active when
  //   stuck, and a tap bumps scrollNonce → we scroll to the bottom below).
  const stick = useRef(true);
  // Transcript is NOT remounted per session (it re-pins via the sessionId
  // effect), so the once-wired scroll listeners would capture a stale
  // sessionId. Read it through a ref that tracks the latest prop.
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  // Bumped by the composer toggle (requestStickToBottom) to ask us to scroll to
  // the bottom now; the effect below reacts to a change.
  const scrollNonce = useScrollNonce(sessionId);

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

  // Wire user-intent listeners ONCE; they read parentRef + sessionIdRef each
  // time so the dependency on the ref's contents stays out of React's eyes.
  useEffect(() => {
    const el = parentRef.current;
    if (!el) return undefined;
    const detach = (): void => {
      if (stick.current) {
        stick.current = false;
        // Reading back history → stop following; the composer toggle goes
        // inactive. setSticky no-ops when already off, so this is cheap even
        // though `wheel`/`scroll` fire often.
        setSticky(sessionIdRef.current, false);
      }
    };
    const onScroll = (): void => {
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      const atBottom = dist < 24;
      if (atBottom && !stick.current) {
        // Manually scrolled all the way back to the bottom → re-stick + the
        // toggle reactivates (REQ-4).
        stick.current = true;
        setSticky(sessionIdRef.current, true);
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
    // A (re)opened session starts pinned + following, regardless of a prior
    // detached state (REQ: default on).
    resetSticky(sessionId);
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

  // Auto-snap (the actual "sticky" behaviour) only on rowCount changes, ONLY if
  // we're still stuck — new streamed content keeps the view pinned to the latest
  // line.
  useLayoutEffect(() => {
    if (!stick.current || rowCount === 0) return;
    virtualizer.scrollToIndex(rowCount - 1, { align: "end" });
  }, [rowCount, virtualizer]);

  // The composer toggle's "catch up" tap bumps scrollNonce → scroll to the
  // bottom and resume following. Guarded so it only fires on an actual bump
  // (not on mount / session switch, where the sessionId effect already pins).
  const lastNonceRef = useRef(scrollNonce);
  useEffect(() => {
    if (scrollNonce === lastNonceRef.current) return;
    lastNonceRef.current = scrollNonce;
    stick.current = true;
    if (rowCount > 0) {
      virtualizer.scrollToIndex(rowCount - 1, { align: "end" });
    }
  }, [scrollNonce, rowCount, virtualizer]);

  return (
    <Box sx={{ flex: 1, position: "relative", overflow: "hidden" }}>
      <Box
        ref={parentRef}
        tabIndex={0}
        sx={{
          height: "100%",
          overflowY: "auto",
          overflowX: "hidden",
          // User-controlled side gutter (px, breakpoint-independent like
          // liveview's reading margin); vertical padding stays responsive.
          px: `${padding}px`,
          py: { xs: 1, sm: 1.5 },
          // `em` multiplier on the reading content only (1 = unchanged). MUI
          // Typography descendants set their own rem size and so stay fixed.
          fontSize: `${fontScale}em`,
          // Prose font-family (unset var → theme font). MUI Typography chrome
          // sets its own family and stays put; code fences are explicitly
          // monospace and are unaffected.
          fontFamily: "var(--cowboy-reading-font, inherit)",
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
      {/* The "scroll to latest" affordance is now the persistent sticky toggle
          in the composer (stickyStore + Composer), not a transient Fab here. */}
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
