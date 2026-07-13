// Paged transcript. One row per canonical `RenderItem`; CSS containment skips
// layout/paint for off-screen rows while preserving DOM and native scroll
// anchoring as streamed markdown / code blocks / images grow.
//
// A JavaScript virtualizer was deliberately removed: unmounting variable-height
// rows breaks the iOS column-reverse anchor and loses local tool/permission-card
// state. Server history paging and live event coalescing bound the expensive
// work without fighting native momentum scrolling.

import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
  Paper,
  Skeleton,
  Stack,
  Tooltip,
  Typography,
  keyframes,
  useTheme,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import {
  Bedtime,
  ChatBubbleOutline,
  CleaningServices,
  Close,
  Code,
  Construction,
  ErrorOutline,
  ExpandLess,
  ExpandMore,
  Folder,
  LightbulbOutlined,
  Refresh,
  Search,
  Stop,
  Terminal,
  UnfoldLess,
  WarningAmberRounded,
} from "@mui/icons-material";
import { CLAUDE_VERBS } from "./claudeVerbs";
import { Markdown } from "./Markdown";
import { inlineTokensToMarkdown } from "./inlineImages";
import { ToolBody, type ToolCtx } from "./tools/registry";
import { COMPACTING_NOTICE, derive, type ContentChunk, type RenderItem } from "./derive";
import type { Envelope, Status } from "./protocol";
import {
  discardMessage,
  loadOlder,
  type QueuedMessage,
  retryMessage,
  send,
  useStoreSelector,
} from "./store";
import { haptic } from "./haptic";
import { useReadingSettings } from "./readingSettings";
import {
  resetSticky,
  setSticky,
  useScrollNonce,
} from "./stickyStore";
import { keyLeavesLatest, wheelLeavesLatest } from "./transcriptFollowIntent";
import { ImageLightbox } from "./_shell";

const EMPTY_OPTIMISTIC_MESSAGES: QueuedMessage[] = [];

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


// Shown when a session has NO messages yet but IS live — a freshly created
// session (`new_session` spawns the agent right away, so it's Running and idle,
// waiting for the first prompt; no history is coming, so the skeleton would be
// wrong and a blank wall reads as broken). Dormant / interrupted / crashed empty
// sessions are NOT handled here — their SessionStatusBar already carries the
// matching "send a message to wake/restart it" line, so this would just double it.
function EmptyTranscript({ provider, cwd }: { provider: string; cwd: string }): React.JSX.Element {
  return (
    <Stack
      // m:auto centers the single child within the column-reverse scroll area.
      sx={{
        m: "auto",
        maxWidth: 360,
        px: 3,
        py: 6,
        alignItems: "center",
        textAlign: "center",
        gap: 1.25,
        color: "text.secondary",
      }}
    >
      <ChatBubbleOutline sx={{ fontSize: 38, opacity: 0.4 }} />
      <Typography variant="body1" sx={{ fontWeight: 600, color: "text.primary" }}>
        Send a message to start
      </Typography>
      <Typography variant="caption" sx={{ lineHeight: 1.55 }}>
        The {provider} agent is ready and waiting — your first message kicks off the turn.
      </Typography>
      {cwd && (
        <Typography
          variant="caption"
          sx={{
            opacity: 0.7,
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            fontSize: "0.72rem",
            wordBreak: "break-all",
          }}
        >
          {cwd}
        </Typography>
      )}
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
const codexPhraseFade = keyframes`
  0%   { opacity: 0.28; transform: translateY(1px); }
  12%  { opacity: 1; transform: translateY(0); }
  88%  { opacity: 1; transform: translateY(0); }
  100% { opacity: 0.28; transform: translateY(-1px); }
`;

// Codex activity uses the smallest useful coding gesture: a prompt chevron
// advances toward a breathing caret. There is no enclosing spinner or badge,
// so it stays calm beside transcript text while remaining provider-specific.
const codexPrompt = keyframes`
  0%, 100% { transform: translateX(0); opacity: 0.55; }
  50%      { transform: translateX(1.5px); opacity: 1; }
`;
const codexCaret = keyframes`
  0%, 45%  { opacity: 1; }
  55%, 100% { opacity: 0.24; }
`;

function CodexWorkcell({ size = 16 }: { size?: number }): React.JSX.Element {
  return (
    <Box
      component="svg"
      viewBox="0 0 18 18"
      aria-hidden
      sx={{
        width: size,
        height: size,
        display: "block",
        flexShrink: 0,
        overflow: "visible",
        color: "primary.main",
        "& .codex-workcell-prompt": { animation: `${codexPrompt} 1.25s ease-in-out infinite` },
        "& .codex-workcell-caret": {
          animation: `${codexCaret} 1.05s steps(1, end) infinite`,
        },
        "@media (prefers-reduced-motion: reduce)": {
          "& .codex-workcell-prompt, & .codex-workcell-caret": { animation: "none" },
        },
      }}
    >
      <path className="codex-workcell-prompt" d="M3.5 5.5 7 9l-3.5 3.5" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
      <path className="codex-workcell-caret" d="M9.5 12.5h5" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
    </Box>
  );
}

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

function CodexThinking(): React.JSX.Element {
  const theme = useTheme();
  const reducedMotion = globalThis.matchMedia?.(
    "(prefers-reduced-motion: reduce)",
  ).matches;
  const phraseMs = 4200;
  const [phraseIndex, setPhraseIndex] = useState(() =>
    Math.floor(Date.now() / phraseMs) % CLAUDE_VERBS.length
  );
  useEffect(() => {
    const id = globalThis.setInterval(() => setPhraseIndex((index) => index + 1), phraseMs);
    return () => globalThis.clearInterval(id);
  }, []);
  const phrase = CLAUDE_VERBS[
    phraseIndex % CLAUDE_VERBS.length
  ] ?? "Thinking";
  const muted = theme.palette.text.secondary;
  const blue = theme.palette.mode === "dark" ? "#8FA8FF" : "#4F6BED";
  const mint = theme.palette.mode === "dark" ? "#62D6BC" : "#168B78";
  return (
    <Stack
      direction="row"
      spacing={0.8}
      alignItems="center"
      aria-label="Codex is working"
      sx={{ py: 0.5, alignSelf: "flex-start", color: "text.secondary" }}
    >
      <CodexWorkcell size={17} />
      <Typography
        key={phraseIndex}
        aria-hidden
        variant="caption"
        sx={{
          fontWeight: 550,
          letterSpacing: "0.015em",
          background: `linear-gradient(100deg, ${muted} 0%, ${muted} 24%, ${blue} 44%, ${mint} 56%, ${muted} 74%, ${muted} 100%)`,
          backgroundSize: "220% 100%",
          WebkitBackgroundClip: "text",
          backgroundClip: "text",
          color: "transparent",
          animation: reducedMotion
            ? "none"
            : `${codexPhraseFade} ${String(phraseMs)}ms ease-in-out, ${shimmer} 3.2s linear infinite`,
          "@media (prefers-reduced-motion: reduce)": {
            animation: "none",
            background: "none",
            color: muted,
            WebkitTextFillColor: muted,
          },
        }}
      >
        {phrase}…
      </Typography>
    </Stack>
  );
}

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

// Each provider keeps its own activity language: Claude's morphing spark, the
// Codex workcell, and a neutral Material fallback for providers without one.
function ThinkingIndicator({
  provider,
}: {
  provider: string;
}): React.JSX.Element {
  if (provider === "claude-code") return <ClaudeThinking />;
  if (provider === "codex") return <CodexThinking />;
  return <DefaultThinking />;
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

// Claude Code's auto-compaction notice (`COMPACTING_NOTICE`, shared with derive):
// when its context window fills, the agent streams a STANDALONE assistant message
// whose entire text is the literal "Compacting..." (its own messageId, no
// `_meta`) while it condenses history, then continues the turn under a fresh
// message. Rendered verbatim it reads as a stray one-word reply, so this widget
// gives it a purpose-built treatment.
//
// True when a message item is exactly that compaction notice — every chunk is
// text and the concatenation trims to the literal. Concatenate (not "first
// chunk") so a split stream still matches.
function isCompactingMessage(chunks: ContentChunk[]): boolean {
  if (chunks.length === 0) return false;
  let text = "";
  for (const c of chunks) {
    if (c.type !== "text") return false;
    text += c.text;
  }
  return text.trim() === COMPACTING_NOTICE;
}

// The fold icon's gentle vertical squeeze — "condensing" made literal. Compositor
// transform only (cheap on mobile); frozen under prefers-reduced-motion.
const fold = keyframes`
  0%, 100% { transform: scaleY(1); }
  50%      { transform: scaleY(0.6); }
`;

// Two states, because the notice is PERSISTED in the transcript and stays in
// scrollback after compaction ends:
//   • active (`active` — it's the live tail and the turn is still busy): a
//     terracotta shimmer + a squeezing fold icon = "condensing right now".
//   • done   (anything followed it / the turn is idle): a calm, static muted
//     "Context compacted" note — no infinite shimmer implying it's still going.
function CompactingWidget({ active }: { active: boolean }): React.JSX.Element {
  const theme = useTheme();
  const muted = theme.palette.text.secondary;
  // Claude Code brand terracotta — this is a CC operation, matching ClaudeThinking.
  const accent = "#D97757";
  const reducedMotion = globalThis.matchMedia?.(
    "(prefers-reduced-motion: reduce)",
  ).matches;
  return (
    <Stack
      direction="row"
      spacing={0.75}
      alignItems="center"
      sx={{
        alignSelf: "flex-start",
        my: 0.25,
        px: 1,
        py: 0.4,
        borderRadius: 1.5,
        border: 1,
        borderColor: active ? alpha(accent, 0.35) : "divider",
        bgcolor: active ? alpha(accent, 0.08) : "action.hover",
      }}
    >
      <UnfoldLess
        aria-hidden
        sx={{
          fontSize: 16,
          color: active ? accent : muted,
          ...(active &&
            !reducedMotion && {
              animation: `${fold} 1.4s ease-in-out infinite`,
            }),
        }}
      />
      {active ? (
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
          Compacting context…
        </Typography>
      ) : (
        <Typography variant="caption" sx={{ color: muted, fontWeight: 500 }}>
          Context compacted
        </Typography>
      )}
    </Stack>
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

function ThoughtSteps({
  sections,
  streaming,
  codex,
}: {
  sections: string[];
  streaming: boolean;
  codex: boolean;
}): React.JSX.Element {
  const visible = sections.filter((section) => section.trim() !== "");
  return (
    <Stack spacing={0} sx={{ flex: 1, minWidth: 0 }} aria-label="Thinking steps">
      {visible.map((section, index) => {
        const current = streaming && index === visible.length - 1;
        const hasNext = index < visible.length - 1;
        return (
          <Box
            // A thought section has no upstream id. Its index is stable because
            // Codex only appends sections while streaming this item.
            key={index}
            sx={{
              position: "relative",
              pl: codex ? 2.75 : 2,
              pr: codex && current ? 1 : 0,
              py: codex && current ? 0.5 : 0,
              mb: hasNext ? (codex ? 0.25 : 0.75) : 0,
              borderRadius: codex && current ? 1.25 : 0,
              bgcolor: codex && current ? "action.hover" : "transparent",
            }}
          >
            {codex
              ? (
                <Box
                  aria-hidden="true"
                  sx={{
                    position: "absolute",
                    left: 3,
                    top: current ? "0.43em" : "0.08em",
                    width: 15,
                    height: 15,
                    display: "grid",
                    placeItems: "center",
                    color: current ? "primary.main" : "text.disabled",
                    animation: current ? `${pulse} 1.5s ease-in-out infinite` : "none",
                    "@media (prefers-reduced-motion: reduce)": { animation: "none" },
                  }}
                >
                  <LightbulbOutlined sx={{ fontSize: 14 }} />
                </Box>
              )
              : (
                <Box
                  aria-hidden="true"
                  sx={{
                    position: "absolute",
                    left: codex ? 7 : 2,
                    top: "0.62em",
                    width: 5,
                    height: 5,
                    borderRadius: codex ? 1 : "50%",
                    bgcolor: current ? "primary.main" : "text.disabled",
                    animation: current ? `${pulse} 1.4s ease-in-out infinite` : "none",
                    "@media (prefers-reduced-motion: reduce)": { animation: "none" },
                  }}
                />
              )}
            {hasNext && (
              <Box
                aria-hidden="true"
                sx={{
                  position: "absolute",
                  left: codex ? 9 : 4,
                  top: codex ? (current ? "calc(0.43em + 15px)" : "calc(0.08em + 15px)") : "calc(0.62em + 6px)",
                  bottom: -2,
                  width: "1px",
                  bgcolor: "divider",
                }}
              />
            )}
            <Box
              sx={{
                opacity: current || !streaming ? 1 : (codex ? 0.55 : 0.68),
                fontStyle: "normal",
                color: current ? "text.primary" : "text.secondary",
                "& p": {
                  m: 0,
                  fontStyle: "normal",
                  fontWeight: current ? 600 : 500,
                  lineHeight: 1.45,
                },
              }}
            >
              <Markdown text={section} />
              {current && !codex && <StreamingCaret />}
            </Box>
          </Box>
        );
      })}
    </Stack>
  );
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
    // ResizeObserver supplies the natural content box after layout. Reading
    // offsetHeight synchronously for every transcript row forced a style/layout
    // flush during startup, especially on slower Mobile devices.
    const ro = new ResizeObserver(([entry]) => {
      if (entry) setOverflowing(entry.contentRect.height > COLLAPSED_BUBBLE_PX + 80);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // Cap from the first frame: short content is unaffected, while a long message
  // cannot flash fully expanded before the observer's initial delivery.
  const clamp = !expanded;
  return (
    <>
      <Box sx={{ position: "relative" }}>
        <Box sx={{ maxHeight: clamp ? COLLAPSED_BUBBLE_PX : "none", overflow: "hidden" }}>
          <Box ref={ref}>{children}</Box>
        </Box>
        {overflowing && clamp && (
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

// An OPTIMISTIC user bubble — shown the instant you hit send (chat), before the
// daemon's user-echo confirms it. Mirrors the real user bubble (right-aligned,
// primary fill). `pending` (<200ms) looks normal so a fast send never flashes;
// `sending` reads unmistakably as in-flight — the bubble dims, a white sweep
// runs across it, AND an explicit "Sending…" spinner line sits below it (a faint
// sweep alone was too easy to miss); `failed` → red edge + Failed + retry /
// discard. Reconciled out by cmid the moment the echo Envelope lands (same
// bubble, seamless — the dim + line vanish as it confirms).
function OptimisticUserBubble({
  sessionId,
  message,
}: {
  sessionId: string;
  message: QueuedMessage;
}): React.JSX.Element {
  const failed = message.status === "failed";
  const sending = message.status === "sending";
  const cmid = message.cmid ?? "";
  return (
    <Stack alignItems="flex-end" spacing={0.25} sx={{ alignSelf: "stretch", maxWidth: "100%" }}>
      <Paper
        variant="outlined"
        sx={{
          position: "relative",
          p: { xs: 1, sm: 1.25 },
          maxWidth: { xs: "88%", sm: "78%" },
          bgcolor: "primary.main",
          color: "primary.contrastText",
          overflow: "hidden",
          // Dim while in flight so a sending bubble is visibly "not yet
          // delivered"; snaps back to full on confirm.
          opacity: sending ? 0.62 : 1,
          transition: "opacity 0.2s ease",
          ...(failed && { borderColor: "error.main" }),
        }}
      >
        <Markdown text={inlineTokensToMarkdown(message.text) || "📎 attachment"} invert />
        {sending && (
          <Box
            aria-hidden
            sx={{
              position: "absolute",
              inset: 0,
              pointerEvents: "none",
              background:
                "linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.34) 50%, transparent 100%)",
              backgroundSize: "200% 100%",
              animation: `${shimmer} 1.6s linear infinite`,
              "@media (prefers-reduced-motion: reduce)": { animation: "none" },
            }}
          />
        )}
      </Paper>
      {sending && (
        <Stack direction="row" spacing={0.5} alignItems="center" sx={{ pr: 0.25 }}>
          <CircularProgress size={11} thickness={5} sx={{ color: "text.secondary" }} />
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            Sending…
          </Typography>
        </Stack>
      )}
      {failed && (
        <Stack direction="row" spacing={0.25} alignItems="center">
          <Typography variant="caption" sx={{ color: "error.main" }}>
            Failed to send
          </Typography>
          <Tooltip title="Retry">
            <IconButton
              size="small"
              aria-label="retry send"
              onClick={(): void => retryMessage(sessionId, cmid)}
            >
              <Refresh fontSize="small" />
            </IconButton>
          </Tooltip>
          <Tooltip title="Discard">
            <IconButton
              size="small"
              aria-label="discard message"
              onClick={(): void => discardMessage(sessionId, cmid)}
            >
              <Close fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>
      )}
    </Stack>
  );
}

function MessageBubble({
  role,
  chunks,
  streaming,
  autoResumed,
}: {
  role: "assistant" | "user";
  chunks: ContentChunk[];
  /** When true, append a blinking caret after the last text chunk to signal
   *  the model is still producing. */
  streaming?: boolean;
  /** This "user" turn is a daemon-issued auto-resume continuation, not something
   *  the human typed — render it as a distinct system note, never a user bubble
   *  (an empty-result continuation re-issues the prompt verbatim, which would
   *  otherwise read as a duplicate). */
  autoResumed?: boolean;
}): React.JSX.Element {
  const mine = role === "user";
  // Claude Code's "Compacting..." auto-compaction notice → purpose-built widget
  // instead of a stray one-word assistant reply. `streaming` (last item + turn
  // busy) means it's condensing right now; otherwise it's a finished record.
  if (!mine && isCompactingMessage(chunks)) {
    return <CompactingWidget active={!!streaming} />;
  }
  const lastChunkIdx = chunks.length - 1;
  const body = chunks.map((c, i) => (
    <Box key={i} sx={{ position: "relative" }}>
      <ChunkView chunk={c} invert={mine && !autoResumed} />
      {streaming && i === lastChunkIdx && c.type === "text" && (
        <StreamingCaret />
      )}
    </Box>
  ));
  // Auto-resume continuation: a right-aligned, muted "↻ Auto-resumed" note with
  // the re-sent prompt below it. It sits on the right (the "my side" rail, since
  // the daemon issues it on the human's behalf) but stays a muted, bordered note
  // — never the primary-filled user bubble — so it never reads as something the
  // human actually typed.
  if (mine && autoResumed) {
    return (
      <Box
        sx={{
          alignSelf: "flex-end",
          maxWidth: { xs: "92%", sm: "80%" },
          display: "flex",
          flexDirection: "column",
          alignItems: "flex-end",
          gap: 0.5,
          py: 0.5,
        }}
      >
        <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
          Auto-resumed the interrupted turn
        </Typography>
        <Box
          sx={{
            width: "100%",
            px: 1.25,
            py: 0.75,
            borderRadius: 1.5,
            border: 1,
            borderColor: "divider",
            bgcolor: "action.hover",
            color: "text.secondary",
            fontSize: 13,
          }}
        >
          {body}
        </Box>
      </Box>
    );
  }
  // Assistant replies render flush in the page (Zed-style): no border, no card
  // background, just markdown flowing inline. The per-item `py` in the
  // transcript row is the only separator between consecutive replies. Only the
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
  // Raw escape hatch: any card can flip to the verbatim input/output JSON,
  // regardless of how the friendly renderer formatted it.
  const [raw, setRaw] = useState(false);
  // On expand, pull the card's TOP into view. In the column-reverse (bottom-
  // anchored) transcript a growing card shifts its top UP — often behind the
  // frosted AppBar — so the start of the output is hidden. `nearest` + the
  // container's scrollPaddingTop reveals the top below the bar; it no-ops when the
  // card is already fully visible (no yank). Double-rAF so it runs AFTER the resize
  // settles and the transcript's own anchor logic has had its frame.
  const cardRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return undefined;
    let inner = 0;
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(() => {
        cardRef.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
      });
    });
    return () => {
      cancelAnimationFrame(outer);
      cancelAnimationFrame(inner);
    };
  }, [open]);
  const hasDetail = item.rawInput !== undefined || item.content !== undefined;
  const running = item.status === "in_progress" || item.status === "pending";
  // The header shows only the first line of the title — a Bash title IS the whole
  // (possibly multi-line) command, which would otherwise blow up the row.
  const headerTitle = (item.title || "").split("\n")[0] || item.title;
  const ctx: ToolCtx = {
    toolName: item.toolName,
    kind: item.toolKind,
    title: item.title,
    rawInput: item.rawInput && typeof item.rawInput === "object" && !Array.isArray(item.rawInput)
      ? (item.rawInput as Record<string, unknown>)
      : {},
    content: item.content,
    running,
  };
  return (
    <Paper
      ref={cardRef}
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
        {...(hasDetail ? { "data-desktop-item-action": "default" } : {})}
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
          {headerTitle}
        </Typography>
        <Chip size="small" color={toolColor(item.status)} label={item.status} variant="outlined" />
        {hasDetail && (open ? <ExpandLess fontSize="medium" /> : <ExpandMore fontSize="medium" />)}
      </Stack>
      {open && hasDetail && (
        <Box sx={{ borderTop: 1, borderColor: "divider", p: 1, bgcolor: "background.paper" }}>
          {/* Formatted ⇄ Raw toggle (top-right) — friendly view by default, the
              verbatim JSON one tap away for when the structured render hides a
              detail. */}
          <Stack direction="row" justifyContent="flex-end" sx={{ mb: 0.5 }}>
            <Box
              role="button"
              tabIndex={0}
              onClick={(): void => setRaw((v) => !v)}
              onKeyDown={(e): void => {
                if (e.key === "Enter" || e.key === " ") setRaw((v) => !v);
              }}
              sx={{
                cursor: "pointer",
                fontSize: 11,
                fontWeight: 600,
                color: raw ? "primary.main" : "text.disabled",
                fontFamily: "ui-monospace, monospace",
                userSelect: "none",
                px: 0.5,
                "&:hover": { color: "primary.main" },
              }}
            >
              {raw ? "↩ Formatted" : "{ } Raw"}
            </Box>
          </Stack>
          {raw
            ? (
              <>
                {item.rawInput !== undefined && (
                  <>
                    <Typography variant="caption" color="text.secondary">Input</Typography>
                    <Markdown text={"```json\n" + JSON.stringify(item.rawInput, null, 2) + "\n```"} />
                  </>
                )}
                {item.content !== undefined && (
                  <>
                    <Typography variant="caption" color="text.secondary">Output</Typography>
                    <Markdown text={"```json\n" + JSON.stringify(item.content, null, 2) + "\n```"} />
                  </>
                )}
              </>
            )
            : <ToolBody ctx={ctx} />}
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
  // The timeline keeps only a compact RECORD marker — the actual decision is taken
  // in the sticky PermissionOverlay floating above the composer (always reachable,
  // auto-collapses on scroll). Pending = an amber "Permission requested · cmd"
  // line; resolved = a subtle italic "Allowed/Rejected · cmd" so a glaring card
  // doesn't sit in the log forever.
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

// `memo`'d: with `derive` memoized upstream, each item keeps a stable identity
// across renders that don't change the timeline (e.g. scroll-threshold or
// permission-sheet state). Without this, any such re-render would re-run the
// markdown parser + syntax highlighter for every visible row — the source of
// the scroll jitter on mobile.
const ItemView = memo(function ItemView({
  item,
  streaming,
  provider,
}: {
  item: RenderItem;
  /** True when this item is the last assistant chunk-bearing item and the
   *  session is still busy. Adds a blinking caret / dots accordingly. */
  streaming?: boolean;
  provider: string;
}): React.JSX.Element | null {
  switch (item.kind) {
    case "message":
      return (
        <MessageBubble
          role={item.role}
          chunks={item.chunks}
          streaming={!!streaming && item.role === "assistant"}
          autoResumed={item.autoResumed === true}
        />
      );
    case "thought":
      return (
        <Box
          sx={{
            color: "text.secondary",
            alignSelf: "stretch",
            maxWidth: "100%",
            px: provider === "codex" ? 0.25 : 0,
          }}
        >
          <Box sx={{ fontSize: "0.84rem", flex: 1, minWidth: 0 }}>
            {/* Empty Codex HTML separators become compact, connected steps. */}
            <ThoughtSteps
              sections={item.sections}
              streaming={!!streaming}
              codex={provider === "codex"}
            />
          </Box>
        </Box>
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
    case "cleared":
      // "Conversation cleared" divider (the Clear action reset the agent's
      // context here). A centered label between two rules — everything above is
      // transcript-only now; the agent starts fresh below.
      return (
        <Stack
          direction="row"
          spacing={1.25}
          alignItems="center"
          sx={{ color: "text.disabled", alignSelf: "stretch", my: 0.5 }}
        >
          <Box sx={{ flex: 1, height: "1px", bgcolor: "divider" }} />
          <Stack direction="row" spacing={0.5} alignItems="center" sx={{ flexShrink: 0 }}>
            <CleaningServices sx={{ fontSize: "0.95rem" }} />
            <Typography variant="caption" sx={{ fontWeight: 600, letterSpacing: 0.3 }}>
              Conversation cleared
            </Typography>
          </Stack>
          <Box sx={{ flex: 1, height: "1px", bgcolor: "divider" }} />
        </Stack>
      );
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
// already surfaced — DEBOUNCED — by the app-level reconnect banner (the shared
// connection store's reconnectBannerThreshold; see store.ts `conn`). This bar
// tracks only the authoritative session status, which a blip never changes.
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

// FREEZE-WHILE-DETACHED anchor (see the scroll effect). Held in a ref so both
// the scroll listener (capture) and the per-chunk timeline layout effect
// (restore) share one anchor. `self` flags our own corrective scrollTop write so
// the listener swallows the scroll event it triggers.
interface FreezeAnchor {
  key: string | null;
  top: number;
  self: boolean;
}

// Snapshot the message row at the viewport CENTRE + its offset from the
// container top. Centre (not the top edge) so the probe always lands on a row,
// never in the container's top padding / status-strip inset.
function captureFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  const r = el.getBoundingClientRect();
  const hit = el.ownerDocument.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  const row = hit?.closest<HTMLElement>("[data-key]");
  if (!row) {
    a.key = null;
    return;
  }
  a.key = row.getAttribute("data-key");
  a.top = row.getBoundingClientRect().top - r.top;
}

// Re-assert the captured anchor's offset. d(anchor.screenTop)/d(scrollTop) = -1
// in any container, so `scrollTop += delta` pulls the anchor back to where it
// was — sign-agnostic, so it works for column-reverse's negative scrollTop too.
// Bottom-bubble growth moves the anchor (→ corrected); a top prepend
// (loadOlder) leaves it put (→ delta≈0, no-op). Sub-pixel deltas are skipped so
// a streamed token doesn't churn a scroll write every frame.
function restoreFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  if (a.key === null) return;
  const row = el.querySelector<HTMLElement>(`[data-key="${CSS.escape(a.key)}"]`);
  if (!row) return;
  const delta = row.getBoundingClientRect().top - el.getBoundingClientRect().top - a.top;
  if (Math.abs(delta) < 0.5) return;
  a.self = true;
  el.scrollTop += delta;
}

/** After a Busy turn has been quiet (no timeline growth) this many minutes, show
 *  a count-up "still waiting" badge. cowboy deliberately does NOT auto-kill a
 *  silent turn — idle time can't distinguish a slow turn from a wedged one (Zed,
 *  the ACP author, reaches the same conclusion), so the human stays the judge:
 *  the badge makes the silence visible and the user recovers manually via Stop. */
const QUIET_BADGE_MIN = 5;

/** Activity signature without serializing tool payloads. A screenshot-bearing
 * tool result can be tens of megabytes; JSON.stringify here used to block the
 * main thread again on every unrelated Transcript render. */
function itemProgressSignature(item: RenderItem | undefined, count: number): string {
  if (!item) return "";
  switch (item.kind) {
    case "message":
      return `${count}:${item.key}:m:${item.chunks.map((chunk) =>
        chunk.type === "text" ? chunk.text.length : chunk.src.length
      ).join(",")}`;
    case "thought":
      return `${count}:${item.key}:t:${item.sections.map((section) => section.length).join(",")}`;
    case "tool":
      return `${count}:${item.key}:tool:${item.status}:${item.title}`;
    case "permission":
      return `${count}:${item.key}:permission:${item.resolved}:${item.chosen ?? ""}`;
    case "lifecycle":
      return `${count}:${item.key}:lifecycle:${item.status}`;
    case "cleared":
      return `${count}:${item.key}:cleared:${item.at}`;
  }
}

/** Whole minutes since `signature` (last-item size + count) last changed — i.e.
 *  since the last streamed activity. Refs are updated during render (derived from
 *  the prop) so there's no frame lag; a coarse 30s tick re-reads the clock. This
 *  is a human-facing minute counter, not precise timing, and never touches state. */
function useQuietMinutes(signature: string): number {
  const changedAt = useRef(Date.now());
  const prevSig = useRef(signature);
  const [, tick] = useState(0);
  if (signature !== prevSig.current) {
    prevSig.current = signature;
    changedAt.current = Date.now();
  }
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);
  return Math.max(0, Math.floor((Date.now() - changedAt.current) / 60_000));
}

export function Transcript({
  sessionId,
  timeline,
  status,
  provider,
  cwd,
  loading,
  connected,
  topInset,
  bottomInset,
  onScrollableChange,
  desktopNavigation = false,
}: {
  sessionId: string;
  timeline: Envelope[];
  status: Status;
  /** Drives the per-provider "thinking" indicator flavor (color + verbs). */
  provider: string;
  /** The session's working directory — shown in the empty-state hint so a brand
   *  new session reads as "ready in <cwd>", not a blank wall. */
  cwd: string;
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
  /** Extra BOTTOM padding for the scroll content (a CSS length), so the newest
   *  message clears the floating glass composer+navbar at rest while still
   *  scrolling UNDER it mid-scroll. column-reverse → padding-bottom is the
   *  visual bottom (newest side). Undefined = none. */
  bottomInset?: string | undefined;
  /** Notified when the scroll container's content starts/stops OVERFLOWING the
   *  viewport (i.e. there's real content that lives behind the floating composer
   *  glass). The composer slab uses it to show its "floating above the scroll"
   *  up-shadow only when something actually scrolls under it — an empty/short
   *  conversation shouldn't cast that shadow. Read-only: never writes scrollTop. */
  onScrollableChange?: ((scrollable: boolean) => void) | undefined;
  /** Desktop-only item navigation. Mobile keeps its touch-first transcript and
   * does not expose rows to the Desktop focus registry. */
  desktopNavigation?: boolean;
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
  // CSS var (useReadingFontFaces). A change re-renders here and contained rows
  // reflow on the next pass, so live adjustments apply without a
  // reload.
  const { padding, lineHeight } = useReadingSettings();
  // A pending tool-permission is surfaced by the sticky PermissionOverlay, which
  // lives in the COMPOSER (sharing the floating slot + mutual-exclusivity with the
  // turn-status pill). The timeline here keeps only a record marker (PermissionCard).
  const busy = status === "busy";
  // "Working" == the ACP turn in flight (`busy`) — the SAME predicate the composer
  // and the status dot use, mirroring Zed's `Generating == running_turn.is_some()`.
  // Deliberately NOT keyed off tool-call status: a `pending`/`in_progress` tool is
  // an unreliable signal — agents never emit `in_progress`, and a dangling `pending`
  // (a lost tool terminal, or a backgrounded sub-agent) survives many later turns,
  // so keying off it latched the "Actioning…" spinner on a FINISHED turn (the
  // reported bug: a green idle dot beside a live spinner). `busy` spans the whole
  // turn — every tool call included, since the ACP `session/prompt` request stays
  // in flight until turn end — so it can't blink out between tools. Require a live
  // connection: while the WS is down the last-known status is stale, so a perpetual
  // spinner is wrong; the ConnectionBanner conveys the disconnect and reconnect
  // re-broadcasts the real status.
  const working = connected && busy;
  // No messages yet + a LIVE session (a freshly created session is Running-idle,
  // waiting for the first prompt) → show the "send a message to start" empty state
  // instead of a blank wall. Non-live empties (exited/interrupted/crashed) are
  // already covered by SessionStatusBar's hint, so they fall through to a plain
  // empty area + that bar (no duplicate).
  const isLive = status === "starting" || status === "running" || status === "busy";
  const lastIdx = items.length - 1;
  const lastItem = lastIdx >= 0 ? items[lastIdx] : undefined;
  // Signature that changes whenever the last item grows (more chunks / text) or a
  // new item is appended — drives the caret idle-cap (Layer 5). Cheap: serializes
  // only the last item.
  const lastSig = itemProgressSignature(lastItem, items.length);
  const quietMin = useQuietMinutes(lastSig);
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
  // Report scroll-overflow (content taller than the viewport) to the parent so
  // the composer slab can gate its up-shadow on real scrollable content. Kept in
  // refs so the once-wired scroll effect calls the latest callback without
  // re-binding, and only fires on a CHANGE (cheap to call every scroll/chunk).
  const onScrollableChangeRef = useRef(onScrollableChange);
  onScrollableChangeRef.current = onScrollableChange;
  const lastScrollableRef = useRef<boolean | null>(null);
  const reportScrollableRef = useRef<() => void>(() => undefined);
  reportScrollableRef.current = (): void => {
    const el = parentRef.current;
    if (!el) return;
    const v = el.scrollHeight > el.clientHeight + 1;
    if (v === lastScrollableRef.current) return;
    lastScrollableRef.current = v;
    onScrollableChangeRef.current?.(v);
  };
  // Shared FREEZE-WHILE-DETACHED anchor (captured by the scroll listener,
  // restored by the per-chunk timeline effect). See the scroll effect below.
  const freezeRef = useRef<FreezeAnchor>({ key: null, top: 0, self: false });
  // History pagination state for this session (from the store): drives the
  // "loading older…" indicator at the top + the reached-start cutoff.
  const paging = useStoreSelector((snapshot) => snapshot.pagination.get(sessionId));

  // Bootstrap history until the viewport is actually filled. Older pages were
  // previously requested only from the scroll listener below; when the initial
  // compacted tail was shorter than one screen there was no overflow, therefore
  // no scroll event, therefore no way to fetch the thousands of older events.
  // Re-check after every page lands and stop as soon as content overflows or the
  // server says we reached the beginning. requestAnimationFrame lets Markdown
  // and the column-reverse flex layout publish their final height first.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (
      !el ||
      !paging ||
      paging.reachedStart ||
      paging.loadingOlder ||
      paging.beforeSeq === null
    ) return undefined;
    const raf = requestAnimationFrame(() => {
      if (el.scrollHeight <= el.clientHeight + 1) {
        void loadOlder(sessionId);
      }
    });
    return () => cancelAnimationFrame(raf);
  }, [items.length, paging?.beforeSeq, paging?.loadingOlder, paging?.reachedStart, sessionId]);
  // This device's optimistic chat sends awaiting the daemon echo — rendered as
  // user bubbles below the latest real item (newest at the very bottom), dropped
  // by cmid the moment the echo lands. Empty in the common (confirmed) case.
  const optimisticMsgs = useStoreSelector(
    (snapshot) => snapshot.optimisticMessages.get(sessionId) ?? EMPTY_OPTIMISTIC_MESSAGES,
  );
  // "stick-to-bottom" UX, done properly this time:
  //
  // Previous bug: we listened to `onScroll` to decide if the user "wanted"
  // the bottom; but `onScroll` ALSO fires for our own `scrollToIndex`. So
  // every streamed chunk: snap-to-bottom → onScroll → atBottom=true →
  // stick=true → user wheels up → next chunk → snap-back. Unkillable.
  //
  // New model:
  // - Treat user intent as a separate signal: a wheel / arrow key moving AWAY
  //   from the bottom, or a touchstart, means "I am leaving the bottom" — set
  //   stick=false *immediately*, before any subsequent programmatic scroll can
  //   confuse us. Bottom-bound wheel/key repeat is ignored so its tail can't
  //   undo the scroll listener's automatic re-enable at the boundary.
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
    // True while a finger is down on the transcript. A scroll-up gesture begins
    // NEAR the bottom, so the very first scroll events still read `fromBottom <
    // 24` — which would re-stick the moment after `touchstart` detached, leaving
    // the toggle stuck "active" all the way up (the reported bug). Gate the
    // re-stick on this so it only fires once the gesture SETTLES at the bottom.
    let touching = false;
    const detach = (): void => {
      if (stick.current) {
        stick.current = false;
        // Reading back history → stop following; the composer toggle goes
        // inactive. setSticky no-ops when already off, so this is cheap even
        // though `wheel`/`scroll` fire often.
        setSticky(sessionIdRef.current, false);
      }
    };

    // FREEZE-WHILE-DETACHED. When the reader is scrolled up (NOT stuck), the
    // newest bubble streaming at the visual bottom grows UPWARD (column-reverse
    // anchors the bottom, so a taller bottom shoves everything — including the
    // reader's view — up). The native bottom-anchor only helps while stuck. To
    // hold a detached view still, element-anchor it: snapshot the message row at
    // the viewport centre + its offset (captureFreezeAnchor) as the reader
    // scrolls, then re-assert that offset on every content change — driven from
    // the per-chunk timeline layout effect below (the container's own RO doesn't
    // fire on content growth) plus this RO for container resizes (keyboard).
    const captureAnchor = (): void => captureFreezeAnchor(el, freezeRef.current);
    const restoreAnchor = (): void => restoreFreezeAnchor(el, freezeRef.current);
    const onTouchStart = (): void => {
      touching = true;
      detach();
    };
    const onTouchEnd = (): void => {
      touching = false;
    };
    const onWheel = (event: WheelEvent): void => {
      // A wheel/trackpad gesture that continues toward the bottom can still
      // emit events after `scroll` has re-enabled Following. Detaching for
      // every wheel event therefore produced an on -> off flicker at the
      // boundary. Only an upward (away-from-latest) gesture expresses intent
      // to pause following.
      if (wheelLeavesLatest(event.deltaY)) detach();
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      // Same boundary rule for keyboard repeat: ArrowDown/PageDown/End/Space
      // may continue firing after reaching the bottom and must not undo the
      // `scroll` handler's automatic re-enable.
      if (keyLeavesLatest(event)) detach();
    };
    const onScroll = (): void => {
      // Our own anchor-restoring scrollTop write re-enters here; swallow it so it
      // neither re-captures (would chase its own correction) nor re-sticks.
      if (freezeRef.current.self) {
        freezeRef.current.self = false;
        return;
      }
      // column-reverse: the bottom is scrollTop 0 (abs handles the sign).
      const fromBottom = Math.abs(el.scrollTop);
      if (fromBottom < 24 && !stick.current && !touching) {
        // Settled back at the bottom (finger up) → re-stick + the toggle
        // reactivates (REQ-4).
        stick.current = true;
        setSticky(sessionIdRef.current, true);
        // Drop the freeze anchor so a later detach captures fresh, not a stale
        // pre-re-stick row (which would yank the view on restore).
        freezeRef.current.key = null;
      }
      // Detached → keep the freeze anchor fresh as the reader scrolls, so the
      // moment they stop, the held position is exactly where they left off.
      if (!stick.current) captureAnchor();
      // Near the top (oldest): prefetch the next older page 2 screens early so
      // the prepend lands before the user reaches it. column-reverse keeps the
      // viewport put when it lands (added at the visual top, away from the bottom
      // anchor), so there's no anchor row to capture. `loadOlder` self-guards.
      const fromTop = el.scrollHeight - el.clientHeight - fromBottom;
      if (fromTop < el.clientHeight * 2) {
        void loadOlder(sessionIdRef.current);
      }
      reportScrollableRef.current();
    };
    el.addEventListener("wheel", onWheel, { passive: true });
    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: true });
    el.addEventListener("touchcancel", onTouchEnd, { passive: true });
    el.addEventListener("scroll", onScroll, { passive: true });
    // Keyboard scrolls (PgUp / arrows) — listen on the container so it must
    // be focused first; that's fine, hits the rare desktop case.
    el.addEventListener("keydown", onKeyDown);
    // Keep pinned when the container resizes UNDER us — the on-screen keyboard
    // opening lifts the composer (via --kb-inset) and the drafts/queue panel
    // expanding shrinks this pane, neither with a doc/selection change to trip
    // the totalSize auto-snap. Only re-pin while still stuck (a scrolled-up
    // reader isn't yanked down). A programmatic scrollTop write doesn't trip
    // `detach` (not a wheel/touch), so this can't fight the user.
    //
    // CONVERGE across a few frames, don't write once: a resize re-renders the
    // newly visible contained rows are laid out over the NEXT frames, so
    // `scrollHeight` is briefly stale and a single
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
      reportScrollableRef.current(); // viewport resized → overflow may have flipped
      if (!stick.current) {
        // Detached: don't follow the bottom — hold the reader's view against the
        // streaming bottom bubble's upward growth (see FREEZE-WHILE-DETACHED).
        restoreAnchor();
        return;
      }
      roTries = 0; // extend the convergence window for this resize burst
      if (roRaf === 0) roRaf = requestAnimationFrame(repin);
    });
    ro.observe(el);
    reportScrollableRef.current(); // initial measure once the container exists
    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchend", onTouchEnd);
      el.removeEventListener("touchcancel", onTouchEnd);
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("keydown", onKeyDown);
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

  // After a timeline change (fires per streamed chunk — `timeline` is a fresh
  // array each envelope). STUCK: follow the bottom, scrollTop 0 (a no-op when the
  // native bottom anchor already held it there, a safety net if it didn't).
  // DETACHED: hold the reader's view against the streaming bottom bubble's upward
  // growth by re-asserting the freeze anchor (column-reverse's native anchor only
  // pins the bottom, which is exactly what a scrolled-up reader does NOT want).
  // Pre-paint (layout effect) so the correction lands without a visible jump.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    if (stick.current) {
      el.scrollTop = 0;
      // Stuck → no live anchor; clear so the next detach captures fresh (never
      // restores to a stale, pre-re-stick position → a jump).
      freezeRef.current.key = null;
    } else if (freezeRef.current.key === null) {
      // Detached without a scroll gesture to capture one (e.g. the composer's
      // sticky toggle) → seed the anchor at the current view this first chunk.
      captureFreezeAnchor(el, freezeRef.current);
    } else {
      restoreFreezeAnchor(el, freezeRef.current);
    }
    // Content grew/shrank → overflow may have flipped, and neither the RO (the
    // viewport didn't resize) nor `scroll` fires. Re-measure AFTER paint: the
    // newly visible contained rows lay out over the next frame, so
    // `scrollHeight` is briefly stale this layout pass.
    const sRaf = requestAnimationFrame(() => reportScrollableRef.current());
    return () => cancelAnimationFrame(sRaf);
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
        // scroll area up, never overlapping the last message). The "loading
        // older" spinner stays absolutely positioned, unaffected.
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
          // on screen; a short transcript sits at the bottom beside the composer,
          // while overflowing content keeps the same bottom-anchor scroll model.
          display: "flex",
          flexDirection: "column-reverse",
          // User-controlled side gutter (px, breakpoint-independent like
          // liveview's reading margin).
          px: `${padding}px`,
          // Vertical padding as EXPLICIT pt/pb — never the `py` shorthand. `py`
          // emits its OWN padding-bottom rule AFTER pt/pb in the emitted stylesheet
          // (same specificity → later rule wins), so it silently overrode the
          // bottomInset pb: the reserved space collapsed to 8px and the floating
          // panel covered the newest messages (the long-standing "padding 不自适应 /
          // 看不到最新消息" bug — the inset was computed right, just clobbered here).
          // Setting pt/pb directly, each falling back to the responsive gutter when
          // there's no inset, removes the conflict. column-reverse → pt is the visual
          // top (oldest, clears the status strip), pb the visual bottom (newest,
          // clears the bottom glass); content still scrolls UNDER both mid-scroll,
          // and growing pb only reflows the scroll RANGE (no container reflow), so
          // the panel can grow (drafts expand) with the newest pinned just above it.
          pt: topInset ? `calc(${topInset} + 8px)` : { xs: 1, sm: 1.5 },
          // So scrollIntoView (e.g. a ToolCard expanding) aligns a row BELOW the
          // frosted AppBar, not under it — content scrolls under the bar otherwise.
          scrollPaddingTop: topInset ? `calc(${topInset} + 8px)` : 8,
          // `--awaiting-h` (set by TurnStatusOverlay, 0 when absent) RESERVES the
          // floating status pill's height so the newest message clears it at rest
          // while the pill stays pinned above the composer (sticky-not-covering).
          pb: bottomInset
            ? `calc(${bottomInset} + var(--awaiting-h, 0px) + 8px)`
            : `calc(var(--awaiting-h, 0px) + 12px)`,
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
        ) : items.length === 0 && isLive ? (
          <EmptyTranscript provider={provider} cwd={cwd} />
        ) : (
          // Rendered NEWEST-FIRST in the DOM; column-reverse flips it to
          // oldest-at-top / newest-at-bottom on screen. The trailing dots are
          // DOM-first → the very bottom (below the newest item). Keyed by the
          // item's STABLE key (first envelope seq) so prepending older history
          // doesn't re-mount/jump rows.
          <>
            {/* Still-waiting row: after QUIET_BADGE_MIN of no timeline activity on a
                working turn, surface the silence (count-up) + a REAL red Stop button.
                cowboy no longer auto-kills a silent turn (see acp.rs) — the human
                decides, so the recovery action is a first-class control here. */}
            {working && quietMin >= QUIET_BADGE_MIN && (
              <Box
                sx={{
                  py: 0.5,
                  display: "flex",
                  justifyContent: "center",
                  alignItems: "center",
                  gap: 1,
                  flexWrap: "wrap",
                }}
              >
                <Typography variant="caption" color="text.secondary">
                  ⏱ 已等待 {quietMin} 分钟无响应
                </Typography>
                <Button
                  size="small"
                  variant="contained"
                  color="error"
                  startIcon={<Stop sx={{ fontSize: 16 }} />}
                  onClick={(): void => {
                    haptic(24); // medium — interrupting a turn is significant
                    send({ type: "cancel", session_id: sessionId });
                  }}
                  sx={{ textTransform: "none", minHeight: 28, py: 0.25 }}
                >
                  中断
                </Button>
              </Box>
            )}
            {showTrailingDots && (
              <Box sx={{ py: 0.625, display: "flex", flexDirection: "column" }}>
                <ThinkingIndicator provider={provider} />
              </Box>
            )}
            {/* Optimistic chat bubbles: newest-first in the DOM (column-reverse →
                they sit just above the latest real item / below the dots). */}
            {optimisticMsgs
              .slice()
              .reverse()
              .map((om) => (
                <Box
                  key={`opt-${om.cmid ?? om.id}`}
                  sx={{ py: 0.625, display: "flex", flexDirection: "column" }}
                >
                  <OptimisticUserBubble sessionId={sessionId} message={om} />
                </Box>
              ))}
            {items
              .map((item, i) => ({ item, i }))
              .reverse()
              .map(({ item, i }) => (
                <Box
                  key={item.key}
                  data-key={item.key}
                  {...(desktopNavigation
                    ? {
                      "data-desktop-item": item.key,
                      tabIndex: -1,
                    }
                    : {})}
                  sx={{
                    py: 0.625,
                    display: "flex",
                    flexDirection: "column",
                    // Do not use content-visibility here. iOS WebKit can retain the
                    // intrinsic height but skip painting a row when it is inside a
                    // column-reverse scroller, leaving a large blank hole until the
                    // user scrolls. Correct transcript rendering is more important
                    // than avoiding paint for off-screen Markdown rows.
                  }}
                >
                  <ItemView
                    item={item}
                    streaming={working && i === lastIdx}
                    provider={provider}
                  />
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
      {/* The scroll-to-latest affordance is the persistent sticky/auto-scroll
          toggle in the composer (stickyStore + Composer), not a pill here. A
          pending permission is surfaced by the PermissionOverlay in the composer. */}
    </Box>
  );
}
