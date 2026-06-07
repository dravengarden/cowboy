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
  Paper,
  Skeleton,
  Stack,
  Typography,
  keyframes,
  useTheme,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import {
  Bedtime,
  Code,
  Construction,
  ErrorOutline,
  ExpandLess,
  ExpandMore,
  Folder,
  Psychology,
  Search,
  Terminal,
  WarningAmberRounded,
} from "@mui/icons-material";
import { CLAUDE_VERBS } from "./claudeVerbs";
import { Markdown } from "./Markdown";
import { derive, type ContentChunk, type RenderItem } from "./derive";
import type { Envelope, Status } from "./protocol";
import { loadOlder, send, useStore } from "./store";
import { useReadingSettings } from "./readingSettings";
import {
  resetSticky,
  setSticky,
  useScrollNonce,
} from "./stickyStore";
import { BottomSheet, ImageLightbox } from "./_shell";

// --- Loading primitives -----------------------------------------------------

// Chat-history skeleton shown while a session's snapshot is still in flight (the
// startup blank). A handful of placeholder turns — assistant blocks read as
// left-aligned prose lines, "mine" blocks as a right-aligned bubble — so it
// reads as a conversation, not a form. Sizes are %/maxWidth-relative and it
// rides inside the transcript's padded reading column, so the same markup gives
// the right gutter + line width on iPhone, iPad and desktop with no breakpoints.
const SKELETON_TURNS: { mine: boolean; lines: string[] }[] = [
  { mine: false, lines: ["92%", "84%", "61%"] },
  { mine: true, lines: ["52%"] },
  { mine: false, lines: ["88%", "96%", "72%", "47%"] },
  { mine: true, lines: ["38%"] },
  { mine: false, lines: ["80%", "65%"] },
];

function TranscriptSkeleton(): React.JSX.Element {
  return (
    <Stack
      spacing={3}
      sx={{ py: 2 }}
      aria-busy="true"
      aria-label="Loading chat history"
    >
      {SKELETON_TURNS.map((turn, i) => (
        <Stack
          // Static placeholder list — index keys are fine (no reordering).
          key={i}
          spacing={0.7}
          sx={{ alignItems: turn.mine ? "flex-end" : "stretch", width: "100%" }}
        >
          {turn.mine ? (
            <Skeleton
              variant="rounded"
              animation="pulse"
              width={turn.lines[0]}
              height={34}
              sx={{ maxWidth: "75%", borderRadius: 2.5 }}
            />
          ) : (
            turn.lines.map((w, j) => (
              <Skeleton key={j} variant="text" animation="pulse" width={w} height={20} />
            ))
          )}
        </Stack>
      ))}
    </Stack>
  );
}


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
  // Claude Code's own terracotta — this indicator is Claude's branded "working"
  // personality (its star glyph + jargon verbs), so it keeps the brand colour.
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
  // The tool-card leading icon tracks the theme accent (`color="primary"`,
  // was the default text colour) and the reading-size setting
  // (`fontSize="inherit"` sizes to the row's em like the title does, instead of
  // a fixed 24px "medium") — so it adapts to both theme and font.
  const props = { fontSize: "inherit" as const, color: "primary" as const };
  switch (kind) {
    case "read":
      return <Folder {...props} />;
    case "edit":
      return <Code {...props} />;
    case "execute":
      return <Terminal {...props} />;
    case "search":
      return <Search {...props} />;
    default:
      return <Construction {...props} />;
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

// A user-message body that collapses when it's very tall — a pasted log / big
// snippet shouldn't flood the transcript (the reported case). Community pattern
// (Claude.ai / ChatGPT / Zed, OneReach's "show more" widget, the CSS-Tricks
// fade-read-more): clamp to a max-height, fade the cut edge to the bubble
// colour, and toggle Show more / Show less. The cap is measured off the NATURAL
// content (the inner ref isn't clamped, so the clamp can never hide its own
// toggle) and re-measured on async growth (a pasted image loading). One height
// cap + a centered text toggle behaves identically on mobile + desktop.
const COLLAPSED_BUBBLE_PX = 200;
function CollapsibleUserBody({
  children,
}: {
  children: React.ReactNode;
}): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return undefined;
    // Only collapse when there's a meaningful amount to hide (a hair over the
    // cap isn't worth a toggle) — natural height must clear the cap + a buffer.
    const measure = (): void => setOverflowing(el.offsetHeight > COLLAPSED_BUBBLE_PX + 80);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const clamp = overflowing && !expanded;
  return (
    <>
      <Box sx={{ position: "relative" }}>
        <Box sx={{ maxHeight: clamp ? COLLAPSED_BUBBLE_PX : "none", overflow: "hidden" }}>
          <Box ref={ref}>{children}</Box>
        </Box>
        {clamp && (
          <Box
            aria-hidden
            sx={{
              position: "absolute",
              left: 0,
              right: 0,
              bottom: 0,
              height: 44,
              pointerEvents: "none",
              background: (t) =>
                `linear-gradient(to bottom, transparent, ${t.palette.primary.main})`,
            }}
          />
        )}
      </Box>
      {overflowing && (
        <Box sx={{ display: "flex", justifyContent: "center", mt: 0.25 }}>
          <Button
            size="small"
            disableRipple
            onClick={(): void => setExpanded((e) => !e)}
            endIcon={expanded ? <ExpandLess /> : <ExpandMore />}
            sx={{
              color: "primary.contrastText",
              textTransform: "none",
              minWidth: 0,
              py: 0,
              opacity: 0.85,
              "& .MuiButton-endIcon": { ml: 0.25 },
              "& .MuiButton-endIcon > svg": { fontSize: 18 },
              "&:hover": { bgcolor: "transparent", opacity: 1 },
            }}
          >
            {expanded ? "Show less" : "Show more"}
          </Button>
        </Box>
      )}
    </>
  );
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
      <CollapsibleUserBody>{body}</CollapsibleUserBody>
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
      elevation={0}
      sx={{
        alignSelf: "stretch",
        overflow: "hidden",
        // Borderless: a filled `background.paper` surface (a touch lighter than
        // the transcript bg) reads as a soft card WITHOUT a 1px outline. The
        // outlined border (theme `divider`) drew a faint violet line top+bottom
        // on every collapsed card, and a tool-heavy transcript stacked them into
        // a "ruled paper" look. The icon + status chip already mark it as a tool.
        bgcolor: "background.paper",
        borderRadius: 1,
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
            // Match the prose typography so a tool card reads as part of the same
            // document, not as smaller UI chrome:
            //  - FAMILY: follow the `--cowboy-reading-font` setting like the prose
            //    does, instead of letting MUI Typography pin the theme font —
            //    otherwise picking a serif/sans reading face restyles the messages
            //    but the tool cards stay on the system font, which reads as
            //    inconsistent. Shell commands (execute) stay monospace: they're
            //    code, and a path full of slashes in a serif is worse, not better.
            //  - SIZE: `inherit` (not body2's fixed 0.875rem) so the title tracks
            //    the transcript's reading-size scale (the scroll container's
            //    `${fontScale}em`) exactly like the markdown body. A fixed rem made
            //    the title visibly smaller than the prose and, worse, it didn't
            //    grow when the reading size was bumped — same family but reading as
            //    two different fonts.
            fontSize: "inherit",
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

// NOTE: the agent's plan is no longer rendered inline here — it's surfaced by
// the pinned, collapsible PlanDock above the composer (src/PlanDock.tsx), so the
// task's progress stays in view without scrolling. See derive.ts `latestPlan`.

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
    case "permission":
      return <PermissionCard item={item} />;
    case "lifecycle": {
      // The interrupted marker (a turn cut off by a daemon restart) is a durable,
      // amber record that stays in the log after the session resumes; crashes are
      // red; everything else is a quiet grey note.
      const interrupted = item.status === "interrupted";
      const color = item.status === "crashed"
        ? "error.main"
        : interrupted
          ? "warning.main"
          : "text.secondary";
      return (
        <Stack direction="row" spacing={1} alignItems="center" sx={{ color }}>
          {interrupted ? (
            <WarningAmberRounded fontSize="medium" />
          ) : (
            <ErrorOutline fontSize="medium" />
          )}
          <Typography variant="caption" sx={{ fontWeight: interrupted ? 600 : 400 }}>
            {item.status}
            {item.detail ? `: ${item.detail}` : ""}
          </Typography>
        </Stack>
      );
    }
  }
});

// A persistent strip pinned at the BOTTOM of the transcript (just above the
// composer), shown only when the session is NOT live. The timeline's lifecycle
// entry scrolls away; this stays put, so an interrupted / crashed / dormant
// session is always visible. Hue matches the navbar StatusDot palette (App.tsx
// statusColor) so the dot and the bar read as one signal. Returns null in the
// live states (running/busy/starting).
//
// Deliberately STATUS-only — it does NOT react to the WS `connected` flag. A
// flaky link (cellular) flaps connected on/off constantly; gating the bar on it
// would strobe a "disconnected" banner and is exactly the kind of transient
// noise that must never distract from a working session. Connection state is
// already surfaced — DEBOUNCED — by the app-level reconnect banner
// (RECONNECT_BANNER_THRESHOLD in store.ts). This bar tracks only the
// authoritative session status, which a blip never changes.
//
// The key signal (the user's ask): a daemon restart that caught a turn in flight
// surfaces as `interrupted` (amber, "didn't finish"), distinct from the normal
// `exited` dormant (grey, "completed, asleep") — completed vs interrupted at a
// glance, no percentage.
function SessionStatusBar({
  status,
}: {
  status: Status;
}): React.JSX.Element | null {
  let tone: "warning" | "error" | "neutral";
  let icon: React.JSX.Element;
  let text: string;
  if (status === "interrupted") {
    tone = "warning";
    icon = <WarningAmberRounded fontSize="small" />;
    text = "Last turn was interrupted before it finished — send a message to start a new one.";
  } else if (status === "crashed") {
    tone = "error";
    icon = <ErrorOutline fontSize="small" />;
    text = "Agent stopped unexpectedly — send a message to restart it.";
  } else if (status === "exited") {
    tone = "neutral";
    icon = <Bedtime fontSize="small" />;
    text = "Session is dormant — send a message to wake it.";
  } else {
    return null; // running / busy / starting → live
  }
  return (
    <Stack
      role="status"
      direction="row"
      spacing={1}
      alignItems="center"
      sx={(theme) => {
        const main =
          tone === "error"
            ? theme.palette.error.main
            : tone === "warning"
              ? theme.palette.warning.main
              : theme.palette.text.disabled;
        return {
          flexShrink: 0,
          px: 1.5,
          py: 0.75,
          borderTop: 1,
          borderColor: alpha(main, 0.4),
          bgcolor: alpha(main, tone === "neutral" ? 0.08 : 0.14),
          color: tone === "neutral" ? theme.palette.text.secondary : main,
        };
      }}
    >
      <Box sx={{ display: "flex", flexShrink: 0 }}>{icon}</Box>
      <Typography
        variant="caption"
        sx={{ fontWeight: 600, minWidth: 0, lineHeight: 1.3 }}
      >
        {text}
      </Typography>
    </Stack>
  );
}

export function Transcript({
  sessionId,
  timeline,
  status,
  provider,
  loading,
  connected,
  topInset,
}: {
  sessionId: string;
  timeline: Envelope[];
  status: Status;
  /** Drives the per-provider "thinking" indicator flavor (color + verbs). */
  provider: string;
  /** True until this session's history snapshot has arrived — show a skeleton
   *  instead of an empty column during the initial load. */
  loading: boolean;
  /** Whether the WS to the daemon is up. The "working" indicator requires it:
   *  while disconnected (e.g. a daemon restart / deploy) the last-known status is
   *  stale and the agent is unreachable, so we must NOT keep spinning "thinking".
   *  The connection banner communicates the disconnect instead. */
  connected: boolean;
  /** Extra top padding for the scroll content (a CSS length), so content clears
   *  the bottom-mode glass status-bar strip at rest. Undefined = none. */
  topInset?: string | undefined;
}): React.JSX.Element {
  // Memoized on `timeline` identity: `applyEnvelope` (store.ts) only hands us a
  // new array when a new event actually lands, so this O(n) fold runs once per
  // event — NOT on every scroll-driven re-render. Stable item identities also
  // let the `memo`'d `ItemView` rows skip re-rendering.
  const items = useMemo(() => derive(timeline), [timeline]);
  // Reader-comfort controls (Settings → Reading). The font-size SCALE is now a
  // GLOBAL app zoom on the root <html> font-size (useGlobalFontScale at the app
  // root), so chrome + prose scale together — it is NOT re-applied here. The
  // reading column inherits it like everything else. `padding` is the column's
  // side gutter; `lineHeight` drives the prose leading (MarkdownImpl `p`
  // inherits it). The reading font-family swaps via the `--cowboy-reading-font`
  // CSS var (useReadingFontFaces). A change re-renders here and the virtualizer
  // re-measures rows on the next pass, so live adjustments reflow without a
  // reload.
  const { padding, lineHeight } = useReadingSettings();
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
  const dead =
    status === "exited" || status === "crashed" || status === "interrupted";
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
  // Require a live connection: while the WS is down (daemon restart / deploy /
  // network drop) the last-known status is stale and the agent is unreachable, so
  // a perpetual "thinking" spinner is wrong (the reported bug — it spun for the
  // whole restart). The connection banner conveys the disconnect instead; on
  // reconnect the daemon re-broadcasts the real status (Exited after a restart).
  const working = connected && (busy || toolInFlight);
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
  const parentRef = useRef<HTMLDivElement>(null);
  // History pagination state for this session (from the store): drives the
  // "loading older…" indicator at the top + the reached-start cutoff.
  const paging = useStore().pagination.get(sessionId);
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

  // SCROLL MODEL: the scroll container is `flex-direction: column-reverse`, so
  // the browser anchors the viewport FROM THE BOTTOM natively. We render rows
  // newest-first in the DOM; column-reverse flips that to oldest-at-top,
  // newest-at-bottom on screen. The payoff is that NO JS ever writes scrollTop
  // to anchor:
  //   - prepend older history → added at the flex-end (visual top), far from the
  //     bottom anchor → the viewport doesn't move. Zero scrollTop write, so
  //     nothing fights iOS momentum and nothing can be a frame late = no jitter.
  //   - stream / append at the bottom while stuck → the bottom anchor (scrollTop
  //     0) holds → follows for free.
  //   - new message while scrolled up → added at the bottom, below the view →
  //     doesn't move it.
  // scrollTop is ONLY written to mean "go to the bottom" (= 0), and only when
  // the user isn't scrolling (session open, catch-up tap, container resize) — so
  // it never collides with an active flick. `Math.abs(scrollTop)` is the
  // distance-from-bottom (Chrome makes scrollTop negative going up; Safari
  // positive — abs handles both).

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
      // column-reverse: the bottom is scrollTop 0 (abs handles the sign).
      const fromBottom = Math.abs(el.scrollTop);
      if (fromBottom < 24 && !stick.current) {
        // Manually scrolled all the way back to the bottom → re-stick + the
        // toggle reactivates (REQ-4).
        stick.current = true;
        setSticky(sessionIdRef.current, true);
      }
      // Near the top (oldest): prefetch the next older page 2 screens early so
      // the prepend lands before the user reaches it. column-reverse keeps the
      // viewport put when it lands (added at the visual top, away from the bottom
      // anchor), so there's no anchor row to capture. `loadOlder` self-guards.
      const fromTop = el.scrollHeight - el.clientHeight - fromBottom;
      if (fromTop < el.clientHeight * 2) {
        void loadOlder(sessionIdRef.current);
      }
    };
    el.addEventListener("wheel", detach, { passive: true });
    el.addEventListener("touchstart", detach, { passive: true });
    el.addEventListener("scroll", onScroll, { passive: true });
    // Keyboard scrolls (PgUp / arrows) — listen on the container so it must
    // be focused first; that's fine, hits the rare desktop case.
    el.addEventListener("keydown", detach);
    // Keep pinned when the container resizes UNDER us — the on-screen keyboard
    // opening lifts the composer (via --kb-inset) and the drafts/queue panel
    // expanding shrinks this pane, neither with a doc/selection change to trip
    // the totalSize auto-snap. Only re-pin while still stuck (a scrolled-up
    // reader isn't yanked down). A programmatic scrollTop write doesn't trip
    // `detach` (not a wheel/touch), so this can't fight the user.
    //
    // CONVERGE across a few frames, don't write once: a resize re-renders the
    // virtualizer's window and react-virtual re-measures the now-visible rows
    // over the NEXT frames, so `scrollHeight` is briefly stale and a single
    // `scrollTop = scrollHeight` lands SHORT — the "expanded drafts and the last
    // message no longer sits at the bottom" report. This is the same lazy-measure
    // gap the session-pin + auto-snap effects already converge across. Coalesce
    // bursts (keyboard animation fires resize every frame) onto one loop, and
    // restart its window on each burst so the final settled frame wins.
    let roRaf = 0;
    let roTries = 0;
    const repin = (): void => {
      roRaf = 0;
      if (!stick.current) return;
      el.scrollTop = 0; // column-reverse: 0 = bottom
      if (++roTries < 5) roRaf = requestAnimationFrame(repin);
    };
    const ro = new ResizeObserver(() => {
      if (!stick.current) return;
      roTries = 0; // extend the convergence window for this resize burst
      if (roRaf === 0) roRaf = requestAnimationFrame(repin);
    });
    ro.observe(el);
    return () => {
      el.removeEventListener("wheel", detach);
      el.removeEventListener("touchstart", detach);
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("keydown", detach);
      ro.disconnect();
      if (roRaf !== 0) cancelAnimationFrame(roRaf);
    };
  }, []);

  // Opening / switching a session always starts pinned to the latest message
  // (chat default). The component isn't remounted per session, so `stick` would
  // otherwise carry over a scrolled-up position. column-reverse already starts
  // at the bottom (scrollTop 0), but a SWITCH from a scrolled-up prior session
  // needs an explicit reset; a few frames cover the initial lazy image/markdown
  // settle (0 stays pinned to the bottom as they grow).
  useLayoutEffect(() => {
    stick.current = true;
    // A (re)opened session starts pinned + following, regardless of a prior
    // detached state (REQ: default on).
    resetSticky(sessionId);
    let raf = 0;
    let tries = 0;
    const pin = (): void => {
      const el = parentRef.current;
      if (el) el.scrollTop = 0; // column-reverse: 0 = bottom
      if (++tries < 5) raf = requestAnimationFrame(pin);
    };
    raf = requestAnimationFrame(pin);
    return () => cancelAnimationFrame(raf);
  }, [sessionId]);

  // FOLLOW after a timeline change. column-reverse makes prepend + scrolled-up
  // append jitter-free natively (see the SCROLL MODEL note above), so the only
  // thing left to assert is "keep following the bottom while stuck" — set
  // scrollTop 0 (a no-op when the native bottom anchor already held it there,
  // a safety net if it didn't). When NOT stuck we never touch scrollTop, so a
  // prepend / new message can't move the reader's view.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (el && stick.current) el.scrollTop = 0;
  }, [timeline]);

  // The composer toggle's "catch up" tap bumps scrollNonce → scroll to the
  // bottom and resume following. Guarded so it only fires on an actual bump
  // (not on mount / session switch, where the sessionId effect already pins).
  const lastNonceRef = useRef(scrollNonce);
  useEffect(() => {
    if (scrollNonce === lastNonceRef.current) return undefined;
    lastNonceRef.current = scrollNonce;
    stick.current = true;
    // Converge across a few frames (column-reverse: 0 = bottom). 0 stays pinned
    // to the bottom as the last bubble's markdown/images settle, so this is
    // really just "re-stick + ensure we're at 0" — the few frames cover the case
    // where the container is mid-resize (keyboard) when the tap lands.
    let raf = 0;
    let tries = 0;
    const pin = (): void => {
      const el = parentRef.current;
      if (el) el.scrollTop = 0;
      if (++tries < 5) raf = requestAnimationFrame(pin);
    };
    raf = requestAnimationFrame(pin);
    return () => cancelAnimationFrame(raf);
  }, [scrollNonce]);

  return (
    <Box
      sx={{
        flex: 1,
        position: "relative",
        overflow: "hidden",
        // Column so the status bar can sit at the bottom IN FLOW (pushing the
        // scroll area up, never overlapping the last message). The permission
        // Fab/sheet stay absolutely positioned, unaffected.
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
      }}
    >
      <Box
        ref={parentRef}
        tabIndex={0}
        sx={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          // column-reverse → the browser anchors from the bottom. Rows are
          // rendered newest-first below and flipped to oldest-top / newest-bottom
          // on screen; a short transcript sits at the bottom (chat convention).
          display: "flex",
          flexDirection: "column-reverse",
          // User-controlled side gutter (px, breakpoint-independent like
          // liveview's reading margin); vertical padding stays responsive.
          px: `${padding}px`,
          py: { xs: 1, sm: 1.5 },
          // Bottom-navbar mode: clear the glass status-bar strip (env safe-area
          // top) at rest. column-reverse → padding-top is the visual top (oldest
          // side); content still scrolls UNDER the strip mid-scroll. Overrides
          // the `py` top only when an inset is supplied.
          ...(topInset && { pt: `calc(${topInset} + 8px)` }),
          // Reading prose line-height. The markdown paragraph renderer inherits
          // this (MarkdownImpl `p`); headings + code keep their own fixed
          // leading, and MUI Typography chrome sets its own, so only body text
          // follows.
          lineHeight,
          // Prose font-family (unset var → theme font). MUI Typography chrome
          // sets its own family and stays put; code fences are explicitly
          // monospace and are unaffected.
          fontFamily: "var(--cowboy-reading-font, inherit)",
          // Hide focus ring; we keep tabIndex for keyboard scroll capture.
          outline: "none",
          overscrollBehavior: "contain",
        }}
      >
        {loading && items.length === 0 ? (
          <TranscriptSkeleton />
        ) : (
          // Rendered NEWEST-FIRST in the DOM; column-reverse flips it to
          // oldest-at-top / newest-at-bottom on screen. The trailing dots are
          // DOM-first → the very bottom (below the newest item). Keyed by the
          // item's STABLE key (first envelope seq) so prepending older history
          // doesn't re-mount/jump rows.
          <>
            {showTrailingDots && (
              <Box sx={{ py: 0.625, display: "flex", flexDirection: "column" }}>
                <ThinkingIndicator provider={provider} />
              </Box>
            )}
            {items
              .map((item, i) => ({ item, i }))
              .reverse()
              .map(({ item, i }) => (
                <Box
                  key={item.key}
                  data-key={item.key}
                  sx={{ py: 0.625, display: "flex", flexDirection: "column" }}
                >
                  <ItemView item={item} streaming={working && i === lastIdx} />
                </Box>
              ))}
          </>
        )}
      </Box>
      {/* "Loading older history" — an ABSOLUTE overlay at the top (not in the
          scroll flow) so it gives feedback without adding height that would
          shift the viewport. Rarely seen thanks to the 2-screen prefetch. */}
      {paging?.loadingOlder && (
        <Box
          sx={{
            position: "absolute",
            top: 8,
            left: "50%",
            transform: "translateX(-50%)",
            zIndex: 2,
            display: "flex",
          }}
        >
          <CircularProgress size={16} thickness={5} sx={{ color: "text.disabled" }} />
        </Box>
      )}
      {/* Persistent bottom strip: interrupted / crashed / dormant / disconnected.
          In-flow (flexShrink:0) so it sits below the scroll area, above the
          composer — never covering the last message. */}
      <SessionStatusBar status={status} />
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
