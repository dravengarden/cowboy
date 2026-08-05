// Paged transcript. One row per canonical `RenderItem`; every row remains in
// normal layout so native scroll anchoring can track streamed markdown / code
// blocks / images as they grow.
//
// A JavaScript virtualizer was deliberately removed: unmounting variable-height
// rows breaks the iOS column-reverse anchor and loses local tool/permission-card
// state. Server history paging and live event coalescing bound the expensive
// work without fighting native momentum scrolling.

import {
  memo,
  startTransition,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  Dialog,
  DialogContent,
  IconButton,
  keyframes,
  Paper,
  Skeleton,
  Stack,
  Tooltip,
  Typography,
  useTheme,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import { DESKTOP_INSET_RADIUS } from "./desktop/DesktopEmbeddedControl";
import { desktopScrollbarSx } from "./desktop/desktopScrollbar";
import { desktopImeOwnsKey } from "./desktop/commands/imeShortcut";
import { sequentialShortcutAvailability } from "./desktop/commands/shortcutAvailability";
import { workspaceCommandKey } from "./desktop/commands/workspaceCommandKey";
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
  KeyboardArrowDown,
  MyLocation,
  NavigateBefore,
  NavigateNext,
  Refresh,
  Search,
  Stop,
  Terminal,
  UnfoldLess,
  WarningAmberRounded,
} from "@mui/icons-material";
import { CLAUDE_VERBS } from "./claudeVerbs";
import { Markdown } from "./Markdown";
import { attachmentDisplayParts } from "./attachments";
import { CodeView, Labeled } from "./tools/blocks";
import { ToolBody, type ToolCtx } from "./tools/registry";
import { toolCopyText, toolHeading, toolUsesRawOnly, toolVariantLabel } from "./tools/presentation";
import { toolRuns, type ToolItem, type ToolRun } from "./tools/runs";
import { formatShellForDisplay } from "./shellFormatter";
import {
  COMPACTING_NOTICE,
  type ContentChunk,
  derive,
  isCompactingTail,
  isCompactionCommandText,
  isCompactionCompletionTail,
  isCompactionCompletionText,
  type RenderItem,
} from "./derive";
import type { Envelope, Status } from "./protocol";
import { TranscriptJudgingActivity } from "./TranscriptTurnActivity";
import {
  canonicalTimeline,
  discardMessage,
  loadOlder,
  loadPreviousQuestionPage,
  type QueuedMessage,
  releaseFollowedHistory,
  retryMessage,
  send,
  useStoreSelector,
} from "./store";
import { importantHaptic, magneticHaptic } from "./haptic";
import { useReadingSettings } from "./readingSettings";
import { mobileTranscriptActivitySurfaceGap } from "./mobileComposerPrimitives";
import {
  requestStickToBottom,
  resetSticky,
  setSticky,
  useScrollNonce,
} from "./stickyStore";
import {
  keyLeavesLatest,
  shouldRestoreDetachedAnchor,
  wheelLeavesLatest,
} from "./transcriptFollowIntent";
import { markTranscriptScrollActivity } from "./transcriptRenderPacing";
import {
  historyPrefetchTransition,
  magneticHapticTransition,
  scrollbackFillRemaining,
  scrollbackReplacementFromTop,
  shouldBackfillTranscriptViewport,
  shouldContinueScrollbackFill,
  shouldRecoverUnrenderableHistory,
  shouldShowFreshSessionEmptyState,
  shouldMagnetizeTranscript,
} from "./transcriptViewport";
import {
  advanceTimelinePresentation,
  revealHistoryPrepend,
} from "./timelinePresentation";
import {
  canRestoreTranscriptViewport,
  getTranscriptViewport,
  saveTranscriptViewport,
} from "./transcriptViewportStore";
import { FloatingActionIsland, ImageLightbox } from "./_shell";
import { Sheet } from "./Sheet";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { DesktopShortcutBar } from "./desktop/DesktopShortcutBar";

const EMPTY_OPTIMISTIC_MESSAGES: QueuedMessage[] = [];
// A byte-bounded history page can contain only a few tall tool/Markdown rows.
// Permit a small chain so a phone viewport is actually filled after opening,
// while retaining a hard ceiling that prevents downloading a whole session.
// Tool-heavy ACP histories can consume several 64-event pages while collapsing
// to only a handful of visible cards. Three pages still left a tall iPad
// viewport half empty. Keep the bootstrap bounded because one history page may
// approach 512 KiB, but allow enough cursor steps to reach useful prose/tool
// boundaries; geometry stops the chain immediately once the viewport fills.
// Pages are fetched one at a time and only while the measured skeleton still
// has meaningful height. This is a safety limit, not a preload target: ordinary
// sessions stop after one or two pages; pathological non-rendering histories
// cannot turn a mount into an unbounded full-log download.
const VIEWPORT_BACKFILL_PAGE_LIMIT = 24;
const VIEWPORT_BACKFILL_SETTLE_MS = 96;
// One upward gesture should reveal a useful reading batch, not three isolated
// tool cards. Count rendered transcript rows rather than assuming a byte page
// has a stable visual density; tool-heavy pages can collapse to very little UI.
const SCROLLBACK_FILL_MINIMUM_ROWS = 10;
const SCROLLBACK_FILL_PAGE_LIMIT = 10;
const SCROLLBACK_FILL_SETTLE_MS = 96;
const SCROLLBACK_IDLE_BOUNDARY_HEIGHT = 132;
const SCROLLBACK_MOUNT_WAIT_MS = 900;

async function waitForScrollbackMount(
  el: HTMLElement,
  previousRowCount: number,
  previousContentHeight: number,
  stillCurrent: () => boolean,
): Promise<void> {
  const deadline = performance.now() + SCROLLBACK_MOUNT_WAIT_MS;
  while (stillCurrent() && performance.now() < deadline) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const bandHeight = el.querySelector<HTMLElement>(
      "[data-transcript-scrollback-fill]",
    )?.getBoundingClientRect().height ?? 0;
    const rowCount = el.querySelectorAll<HTMLElement>("[data-key]").length;
    const contentHeight = Math.max(0, el.scrollHeight - bandHeight);
    if (rowCount > previousRowCount || contentHeight > previousContentHeight + 1) {
      return;
    }
    await new Promise<void>((resolve) => globalThis.setTimeout(resolve, 32));
  }
}

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

const LOADING_FILL_TURNS: { mine: boolean; lines: string[] }[] = [
  { mine: false, lines: ["84%", "63%"] },
  { mine: true, lines: ["46%"] },
  { mine: false, lines: ["91%", "72%", "54%"] },
  { mine: false, lines: ["76%", "58%"] },
  { mine: true, lines: ["39%"] },
  { mine: false, lines: ["88%", "69%", "45%"] },
];

function TranscriptSkeleton({
  desktop,
  provider,
}: {
  desktop: boolean;
  provider: string;
}): React.JSX.Element {
  const [stalled, setStalled] = useState(false);
  useEffect(() => {
    const timer = globalThis.setTimeout(() => setStalled(true), 8_000);
    return () => globalThis.clearTimeout(timer);
  }, []);

  const agent = provider === "claude-code" || provider === "claude-deepseek"
    ? "Claude Code"
    : provider === "gemini"
    ? "Gemini"
    : "Codex";

  return (
    <Stack
      data-transcript-switch-skeleton
      spacing={desktop ? 3 : 2.25}
      sx={{
        minHeight: desktop ? undefined : "100%",
        py: desktop ? 2 : { xs: 2, sm: 3 },
        justifyContent: desktop ? undefined : { xs: "flex-end", sm: "flex-start" },
      }}
      aria-busy="true"
      aria-label="Loading chat history"
    >
      {!desktop && (
        <Stack direction="row" spacing={1} alignItems="center" sx={{ color: "text.secondary" }}>
          <CircularProgress size={14} thickness={4} color="inherit" aria-hidden />
          <Typography variant="caption" sx={{ fontWeight: 650 }}>
            Restoring {agent} conversation…
          </Typography>
        </Stack>
      )}
      {SKELETON_TURNS.map((turn, i) => (
        <Stack
          // Static placeholder list — index keys are fine (no reordering).
          key={i}
          spacing={0.7}
          sx={{ alignItems: turn.mine ? "flex-end" : "stretch", width: "100%" }}
        >
          {turn.mine
            ? (
              <Skeleton
                variant="rounded"
                animation="pulse"
                width={turn.lines[0]}
                height={34}
                sx={{ maxWidth: "75%", borderRadius: 2.5 }}
              />
            )
            : (
              turn.lines.map((w, j) => (
                <Skeleton
                  key={j}
                  variant="text"
                  animation="pulse"
                  width={w}
                  height={20}
                />
              ))
            )}
        </Stack>
      ))}
      {stalled && (
        <Button
          size="small"
          variant="text"
          onClick={(): void => globalThis.location.reload()}
          sx={{ alignSelf: "flex-start", minHeight: 36, textTransform: "none" }}
        >
          Taking a while — reload
        </Button>
      )}
    </Stack>
  );
}

// During the first older-page restore, turn otherwise ambiguous unused space
// into a quiet loading outline. Never show it merely because the agent is
// working: the real status/thinking/tool rows already describe execution, and
// labelling that state "Loading conversation data" leaves a permanent-looking
// skeleton above an already-hydrated transcript. This is a FLEX filler, not
// guessed history height: it consumes only free viewport space, shrinks
// one-for-one as real rows grow, and reaches zero before the transcript
// overflows. It therefore never adds scroll range or disturbs iOS' anchor.
function TranscriptLoadingFill({
  label,
  paused = false,
  onContinue,
}: {
  label: string;
  paused?: boolean;
  onContinue?: (() => void) | undefined;
}): React.JSX.Element {
  return (
    <Box
      data-transcript-loading-fill
      role="status"
      aria-live="polite"
      aria-label={label}
      sx={{
        pointerEvents: paused ? "auto" : "none",
        userSelect: "none",
        WebkitUserSelect: "none",
        position: "relative",
        minHeight: 0,
        overflow: "hidden",
      }}
    >
      <Stack
        sx={{
          position: "absolute",
          inset: "12px 20px",
          mx: "auto",
          maxWidth: 560,
          minHeight: 0,
          justifyContent: "space-evenly",
          gap: 1.1,
          opacity: 0.62,
          "@media (min-width: 600px)": {
            inset: "20px 32px",
            maxWidth: 760,
            // The filler owns exactly the otherwise-empty transcript space.
            // Spread enough conversation-shaped rows through that area on
            // iPad instead of leaving one small cluster floating in its centre.
            justifyContent: "space-evenly",
            gap: 1.5,
          },
          maskImage:
            "linear-gradient(to bottom, transparent 0%, black 12%, black 88%, transparent 100%)",
          WebkitMaskImage:
            "linear-gradient(to bottom, transparent 0%, black 12%, black 88%, transparent 100%)",
        }}
      >
        <Stack direction="row" spacing={1} alignItems="center">
          {paused
            ? null
            : (
              <CircularProgress
                size={13}
                thickness={4}
                color="inherit"
                aria-hidden
              />
            )}
          <Typography
            variant="caption"
            sx={{ color: "text.secondary", fontWeight: 600 }}
          >
            {label}
          </Typography>
          {paused && onContinue && (
            <Button
              size="small"
              variant="text"
              onClick={onContinue}
              sx={{ minHeight: 34, textTransform: "none" }}
            >
              Continue
            </Button>
          )}
        </Stack>
        {LOADING_FILL_TURNS.map(({ mine, lines }, turn) => (
          <Stack
            // Static outline with no reordering.
            key={turn}
            spacing={0.8}
            sx={{
              width: mine ? "58%" : "100%",
              alignSelf: mine ? "flex-end" : "stretch",
              p: mine ? 1.15 : 0,
              borderRadius: mine ? 2.5 : 0,
              "@media (min-width: 600px)": {
                width: mine ? "44%" : "100%",
                maxWidth: mine ? 360 : "none",
                p: mine ? 1.25 : 0,
              },
              bgcolor: mine
                ? (theme) => alpha(theme.palette.primary.main, 0.045)
                : "transparent",
            }}
          >
            {lines.map((width) => (
              <Skeleton
                key={width}
                variant="rounded"
                animation={false}
                width={width}
                height={9}
                sx={{
                  borderRadius: 99,
                  bgcolor: (theme) =>
                    alpha(theme.palette.text.secondary, 0.105),
                }}
              />
            ))}
          </Stack>
        ))}
      </Stack>
    </Box>
  );
}

// In ordinary scrollback the unloaded area belongs at the visual top of the
// transcript, not in a floating overlay. This bounded in-flow outline is
// replaced from its lower edge by real older rows; column-reverse keeps the
// reader's current item anchored while the history grows above it.
function ScrollbackLoadingSkeleton({
  height,
  loading,
  failed,
  onRetry,
}: {
  height: number;
  loading: boolean;
  failed: boolean;
  onRetry: () => void;
}): React.JSX.Element {
  const rows = ["58%", "42%", "51%"];
  return (
    <Box
      data-transcript-scrollback-fill
      role="status"
      aria-live="polite"
      aria-label="Loading earlier messages"
      sx={{
        height: `${Math.max(0, height)}px`,
        minHeight: 0,
        overflow: "hidden",
        pointerEvents: failed ? "auto" : "none",
        userSelect: "none",
        WebkitUserSelect: "none",
        transition: "height 160ms ease-out, opacity 140ms ease",
        opacity: failed ? 1 : loading ? 0.68 : 0.38,
      }}
    >
      {failed
        ? (
          <Button
            size="small"
            variant="text"
            onClick={onRetry}
            sx={{
              width: "100%",
              height: "100%",
              minHeight: 32,
              textTransform: "none",
              color: "text.secondary",
            }}
          >
            Retry earlier messages
          </Button>
        )
        : (
          <Stack
            spacing={1.15}
            sx={{
              height: "100%",
              justifyContent: "flex-end",
              py: 0.75,
              maskImage:
                "linear-gradient(to bottom, transparent 0%, black 24%, black 100%)",
              WebkitMaskImage:
                "linear-gradient(to bottom, transparent 0%, black 24%, black 100%)",
            }}
          >
            {rows.map((title, index) => (
              <Stack key={title} spacing={0.55}>
                <Skeleton
                  variant="text"
                  animation={loading && index >= rows.length - 2 ? "wave" : false}
                  width={title}
                  height={13}
                  sx={{ ml: 1.5, transform: "none" }}
                />
                {loading && (
                  <Skeleton
                    variant="rounded"
                    animation={index >= rows.length - 2 ? "wave" : false}
                    width="100%"
                    height={44}
                    sx={{ borderRadius: 1.75 }}
                  />
                )}
              </Stack>
            ))}
          </Stack>
        )}
    </Box>
  );
}

// Shown when a session has NO messages yet but IS live — a freshly created
// session (`new_session` spawns the agent right away, so it's Running and idle,
// waiting for the first prompt; no history is coming, so the skeleton would be
// wrong and a blank wall reads as broken). Dormant / interrupted / crashed empty
// sessions are NOT handled here — their SessionStatusBar already carries the
// matching "send a message to wake/restart it" line, so this would just double it.
function EmptyTranscript(
  { provider, cwd }: { provider: string; cwd: string },
): React.JSX.Element {
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
      <Typography
        variant="body1"
        sx={{ fontWeight: 600, color: "text.primary" }}
      >
        Send a message to start
      </Typography>
      <Typography variant="caption" sx={{ lineHeight: 1.55 }}>
        The {provider}{" "}
        agent is ready and waiting — your first message kicks off the turn.
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

const prepareSweep = keyframes`
  0% { transform: translateX(-110%); }
  55%, 100% { transform: translateX(310%); }
`;

function PreparingTranscript(
  { provider, cwd }: { provider: string; cwd: string },
): React.JSX.Element {
  return (
    <Stack
      role="status"
      aria-live="polite"
      sx={{
        m: "auto",
        width: "min(360px, calc(100% - 48px))",
        px: 1,
        py: 6,
        alignItems: "center",
        textAlign: "center",
        gap: 1.25,
        color: "text.secondary",
      }}
    >
      <Box
        sx={{
          width: 42,
          height: 42,
          borderRadius: "50%",
          border: 1,
          borderColor: "divider",
          display: "grid",
          placeItems: "center",
          bgcolor: (theme) => alpha(theme.palette.primary.main, 0.08),
        }}
      >
        <Terminal sx={{ fontSize: 21, color: "primary.main" }} />
      </Box>
      <Typography variant="body1" sx={{ fontWeight: 650, color: "text.primary" }}>
        Preparing this session
      </Typography>
      <Typography variant="caption" sx={{ lineHeight: 1.55 }}>
        Creating the workspace and starting {provider}. You can write your first
        message while Cowboy connects everything.
      </Typography>
      <Box
        aria-hidden
        sx={{
          position: "relative",
          width: "min(240px, 72vw)",
          height: 3,
          mt: 0.5,
          overflow: "hidden",
          borderRadius: 99,
          bgcolor: "action.selected",
          "&::after": {
            content: '""',
            position: "absolute",
            inset: 0,
            width: "38%",
            borderRadius: "inherit",
            bgcolor: "primary.main",
            animation: `${prepareSweep} 1.65s cubic-bezier(.4,0,.2,1) infinite`,
          },
          "@media (prefers-reduced-motion: reduce)": {
            "&::after": { animation: "none", width: "55%", opacity: 0.72 },
          },
        }}
      />
      {cwd && (
        <Typography
          variant="caption"
          sx={{
            mt: 0.25,
            maxWidth: "100%",
            opacity: 0.68,
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            fontSize: "0.72rem",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
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

const toolLocateFlash = keyframes`
  0% { box-shadow: 0 0 0 0 transparent; }
  18% { box-shadow: 0 0 0 3px currentColor; }
  100% { box-shadow: 0 0 0 0 transparent; }
`;

// Claude Code's "prompt keyword shimmer": a highlight band sweeps across the
// verb word (color applied via background-clip:text in sx).
const shimmer = keyframes`to { background-position: -200% 0; }`;
// The active thought line is itself live state, not merely prose. A restrained
// colour band moving through the glyphs makes that clear without adding another
// spinner beside the existing work icon. Travel from 100% to 0% while the image
// remains wider than the glyph run: the default repeating 110% → -110% motion
// brought an adjacent gradient tile through the text as a second flash.
const thoughtTextShimmer = keyframes`
  from { background-position: 100% 0; }
  to   { background-position: 0% 0; }
`;
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

const CODEX_PHRASE_MS = 4200;

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
        "& .codex-workcell-prompt": {
          animation: `${codexPrompt} ${
            String(CODEX_PHRASE_MS)
          }ms ease-in-out infinite`,
        },
        "& .codex-workcell-caret": {
          animation: `${codexCaret} ${
            String(CODEX_PHRASE_MS)
          }ms ease-in-out infinite`,
        },
        "@media (prefers-reduced-motion: reduce)": {
          "& .codex-workcell-prompt, & .codex-workcell-caret": {
            animation: "none",
          },
        },
      }}
    >
      <path
        className="codex-workcell-prompt"
        d="M3.5 5.5 7 9l-3.5 3.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        className="codex-workcell-caret"
        d="M9.5 12.5h5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
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
          background:
            `linear-gradient(90deg, ${muted} 0%, ${muted} 40%, ${accent} 50%, ${muted} 60%, ${muted} 100%)`,
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
  const [phraseIndex, setPhraseIndex] = useState(() =>
    Math.floor(Date.now() / CODEX_PHRASE_MS) % CLAUDE_VERBS.length
  );
  useEffect(() => {
    const id = globalThis.setInterval(
      () => setPhraseIndex((index) => index + 1),
      CODEX_PHRASE_MS,
    );
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
          background:
            `linear-gradient(100deg, ${muted} 0%, ${muted} 24%, ${blue} 44%, ${mint} 56%, ${muted} 74%, ${muted} 100%)`,
          backgroundSize: "220% 100%",
          WebkitBackgroundClip: "text",
          backgroundClip: "text",
          color: "transparent",
          animation: reducedMotion
            ? "none"
            : `${codexPhraseFade} ${
              String(CODEX_PHRASE_MS)
            }ms ease-in-out, ${shimmer} 3.2s linear infinite`,
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
  if (provider === "claude-code" || provider === "claude-deepseek") return <ClaudeThinking />;
  if (provider === "codex" || provider === "codex-deepseek") return <CodexThinking />;
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

function isCompactionCommand(chunks: ContentChunk[]): boolean {
  if (chunks.length === 0 || chunks.some((chunk) => chunk.type !== "text")) {
    return false;
  }
  const command = chunks.map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("").trim();
  return isCompactionCommandText(command);
}

function isCompactionCompletion(chunks: ContentChunk[]): boolean {
  if (chunks.length === 0 || chunks.some((chunk) => chunk.type !== "text")) {
    return false;
  }
  const text = chunks.map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("");
  return isCompactionCompletionText(text);
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
function CompactingWidget({
  active,
  provider,
  desktop,
}: {
  active: boolean;
  provider: string;
  desktop: boolean;
}): React.JSX.Element {
  const theme = useTheme();
  const muted = theme.palette.text.secondary;
  const accent = provider === "claude-code"
    ? "#D97757"
    : provider === "claude-deepseek"
    ? "#4D6BFE"
    : theme.palette.primary.main;
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
        my: desktop ? 0.25 : 0.5,
        px: desktop ? 1 : 0.25,
        py: desktop ? 0.4 : 0.5,
        borderRadius: 1.5,
        border: desktop ? 1 : 0,
        borderColor: active ? alpha(accent, 0.35) : "divider",
        bgcolor: desktop
          ? active ? alpha(accent, 0.08) : "action.hover"
          : "transparent",
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
      {active
        ? (
          <Typography
            variant="caption"
            sx={{
              fontWeight: 500,
              letterSpacing: 0.2,
              background:
                `linear-gradient(90deg, ${muted} 0%, ${muted} 40%, ${accent} 50%, ${muted} 60%, ${muted} 100%)`,
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
        )
        : (
          <Typography variant="caption" sx={{ color: muted, fontWeight: 500 }}>
            Context compacted
          </Typography>
        )}
    </Stack>
  );
}

function toolColor(
  status: string,
): "default" | "success" | "error" | "warning" {
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
function TranscriptImage(
  { src, alt }: { src: string; alt: string },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const openTap = useReliableTouchTap<HTMLImageElement>(() => setOpen(true));
  return (
    <>
      <Box
        component="img"
        src={src}
        alt={alt}
        loading="lazy"
        {...openTap}
        sx={{
          maxWidth: "min(360px, 100%)",
          // A portrait screenshot must remain a preview, not become a second
          // full-height viewport inside the conversation. The shared lightbox
          // owns full-resolution reading after a tap.
          maxHeight: {
            xs: "min(38dvh, 320px)",
            sm: "min(48dvh, 420px)",
            md: "min(55vh, 480px)",
          },
          objectFit: "contain",
          display: "block",
          borderRadius: 1,
          my: 0.5,
          mx: "auto",
          cursor: "zoom-in",
          touchAction: "manipulation",
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
  const touchSurface = useSurfaceProfile().kind !== "desktop";
  if (chunk.type === "image") {
    return <TranscriptImage src={chunk.src} alt={chunk.alt ?? ""} />;
  }
  return <Markdown text={chunk.text} invert={invert} touchWrap={touchSurface} />;
}

function ThoughtSteps({
  sections,
  streaming,
  codex,
  touch,
}: {
  sections: string[];
  streaming: boolean;
  codex: boolean;
  touch: boolean;
}): React.JSX.Element {
  const visible = sections.filter((section) => section.trim() !== "");
  return (
    <Stack
      spacing={0}
      sx={{
        flex: 1,
        minWidth: 0,
        // The ordinary provider working line includes 6px of visible breathing
        // space before the Mobile Composer boundary. A live thought has a tinted
        // final surface, so its background otherwise ends only at the timeline
        // row's 5px padding and reads as stuck to the hairline. Match the working
        // line without loosening completed transcript rows or Desktop density.
        pb: touch && streaming
          ? `${mobileTranscriptActivitySurfaceGap}px`
          : 0,
      }}
      aria-label="Thinking steps"
    >
      {
        /* ACP starts a fresh thought item whenever reasoning resumes after a
          tool call.  That boundary is useful to the renderer, but it is not a
          user-facing section: labelling every completed item "Reasoning"
          produces a wall of duplicate headings in tool-heavy turns.  Keep the
          status label only on the one live thought; completed thoughts already
          have their lightbulb + meaningful step text. */
      }
      {codex && streaming && (
        <Stack
          direction="row"
          alignItems="center"
          spacing={0.75}
          sx={{ minHeight: 22, mb: 0.25, color: "text.secondary" }}
          aria-label={streaming ? "Codex is thinking" : "Codex reasoning"}
        >
          <CodexWorkcell size={14} />
          <Typography
            variant="caption"
            sx={{
              fontWeight: 650,
              letterSpacing: "0.025em",
              ...(streaming && {
                backgroundImage: (theme) => {
                  const quiet = theme.palette.text.secondary;
                  const primary = theme.palette.primary.main;
                  return `linear-gradient(100deg, ${quiet} 0%, ${quiet} 35%, ${primary} 50%, ${quiet} 65%, ${quiet} 100%)`;
                },
                backgroundSize: "220% 100%",
                backgroundRepeat: "no-repeat",
                WebkitBackgroundClip: "text",
                backgroundClip: "text",
                color: "transparent",
                animation: `${thoughtTextShimmer} 3.1s linear infinite`,
                "@media (prefers-reduced-motion: reduce)": {
                  animation: "none",
                  backgroundImage: "none",
                  color: "text.secondary",
                },
              }),
            }}
          >
            Thinking
          </Typography>
        </Stack>
      )}
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
                    animation: current
                      ? `${pulse} 1.5s ease-in-out infinite`
                      : "none",
                    "@media (prefers-reduced-motion: reduce)": {
                      animation: "none",
                    },
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
                    animation: current
                      ? `${pulse} 1.4s ease-in-out infinite`
                      : "none",
                    "@media (prefers-reduced-motion: reduce)": {
                      animation: "none",
                    },
                  }}
                />
              )}
            {hasNext && (
              <Box
                aria-hidden="true"
                sx={{
                  position: "absolute",
                  left: codex ? 9 : 4,
                  top: codex
                    ? (current ? "calc(0.43em + 15px)" : "calc(0.08em + 15px)")
                    : "calc(0.62em + 6px)",
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
                ...(codex && current && {
                  backgroundImage: (theme) => {
                    const quiet = theme.palette.text.secondary;
                    const primary = theme.palette.primary.main;
                    const accent = theme.palette.mode === "dark"
                      ? "#62D6BC"
                      : "#168B78";
                    return `linear-gradient(100deg, ${quiet} 0%, ${quiet} 34%, ${primary} 46%, ${accent} 54%, ${quiet} 66%, ${quiet} 100%)`;
                  },
                  backgroundSize: "240% 100%",
                  backgroundRepeat: "no-repeat",
                  WebkitBackgroundClip: "text",
                  backgroundClip: "text",
                  color: "transparent",
                  animation: `${thoughtTextShimmer} 3.1s linear infinite`,
                  // Markdown elements set their own text colour. Inherit the
                  // transparent foreground so the parent gradient clips through
                  // the actual glyphs instead of disappearing behind body text.
                  "& p, & p *": { color: "inherit" },
                  "@media (prefers-reduced-motion: reduce)": {
                    animation: "none",
                    backgroundImage: "none",
                    color: "text.primary",
                    WebkitTextFillColor: "currentColor",
                  },
                }),
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
  containsImage = false,
}: {
  children: React.ReactNode;
  containsImage?: boolean;
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
      if (entry) {
        setOverflowing(entry.contentRect.height > COLLAPSED_BUBBLE_PX + 80);
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // Cap from the first frame: short content is unaffected, while a long message
  // cannot flash fully expanded before the observer's initial delivery.
  // Images are already viewport-bounded thumbnails. Clamping their parent to
  // the text-only 200px cap is what made a portrait screenshot look like it
  // never expanded, so image messages keep their full preview.
  const clamp = !expanded && !containsImage;
  return (
    <>
      <Box sx={{ position: "relative" }}>
        <Box
          sx={{
            maxHeight: clamp ? COLLAPSED_BUBBLE_PX : "none",
            overflow: "hidden",
          }}
        >
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
      {overflowing && !containsImage && (
        <Box sx={{ display: "flex", justifyContent: "center", mt: 0.25 }}>
          <Button
            size="small"
            disableRipple
            onClick={(): void =>
              setExpanded((e) => !e)}
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
  const content = attachmentDisplayParts(message.text, message.attachments);
  return (
    <Stack
      alignItems="flex-end"
      spacing={0.25}
      sx={{ alignSelf: "stretch", maxWidth: "100%" }}
    >
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
        {content.length === 0
          ? <Markdown text="📎 attachment" invert />
          : content.map((part, index) =>
            part.type === "text"
              ? <Markdown key={`text-${index}`} text={part.text} invert />
              : part.attachment.isImage && part.attachment.previewUrl
              ? (
                <TranscriptImage
                  key={`attachment-${index}-${part.attachment.id}`}
                  src={part.attachment.previewUrl}
                  alt={part.attachment.name}
                />
              )
              : (
                <Typography
                  key={`attachment-${index}-${part.attachment.id}`}
                  variant="body2"
                >
                  📎 {part.attachment.name}
                </Typography>
              )
          )}
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
        <Stack
          direction="row"
          spacing={0.5}
          alignItems="center"
          sx={{ pr: 0.25 }}
        >
          <CircularProgress
            size={11}
            thickness={5}
            sx={{ color: "text.secondary" }}
          />
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
  provider,
  desktop,
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
  provider: string;
  desktop: boolean;
}): React.JSX.Element | null {
  const mine = role === "user";
  // Claude Code's "Compacting..." auto-compaction notice → purpose-built widget
  // instead of a stray one-word assistant reply. `streaming` (last item + turn
  // busy) means it's condensing right now; otherwise it's a finished record.
  if (!mine && isCompactingMessage(chunks)) {
    return (
      <CompactingWidget
        active={!!streaming}
        provider={provider}
        desktop={desktop}
      />
    );
  }
  if (!mine && isCompactionCompletion(chunks)) {
    return (
      <CompactingWidget active={false} provider={provider} desktop={desktop} />
    );
  }
  if (mine && isCompactionCommand(chunks)) {
    // The live-edge CompactingWidget is the one lifecycle surface on both
    // products. A second slash-command chip made Desktop diverge from Mobile.
    return null;
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
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ fontWeight: 600 }}
        >
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
      <Box
        sx={{ alignSelf: "stretch", maxWidth: "100%", color: "text.primary" }}
      >
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
      <CollapsibleUserBody
        containsImage={chunks.some((chunk) => chunk.type === "image")}
      >
        {body}
      </CollapsibleUserBody>
    </Paper>
  );
}

function ToolCard({
  item,
  desktop,
  selected,
  onOpen,
}: {
  item: Extract<RenderItem, { kind: "tool" }>;
  desktop: boolean;
  selected: boolean;
  onOpen: (key: string) => void;
}): React.JSX.Element {
  const hasDetail = item.rawInput !== undefined || item.content !== undefined;
  const openDetail = (): void => {
    if (hasDetail) onOpen(item.key);
  };
  const openTap = useReliableTouchTap<HTMLDivElement>(openDetail);
  const running = item.status === "in_progress" || item.status === "pending";
  // The header shows only the first line of the title — a Bash title IS the whole
  // (possibly multi-line) command, which would otherwise blow up the row.
  const fallbackTitle = (item.title || "").split("\n")[0] || item.title;
  const [headerTitle, setHeaderTitle] = useState(fallbackTitle);
  useEffect(() => {
    setHeaderTitle(fallbackTitle);
    if (item.toolKind !== "execute" || !item.rawInput || typeof item.rawInput !== "object") return;
    const input = item.rawInput as Record<string, unknown>;
    const raw = input["command"] ?? input["cmd"];
    const command = typeof raw === "string"
      ? raw
      : Array.isArray(raw)
      ? raw.map(String).join(" ")
      : "";
    // Only pay the async parser cost when the collapsed ACP title exposes a
    // shell launcher. The mvdan/sh formatter unwraps the same launcher in Tool
    // details; using its result here keeps summary and detail on one semantic
    // layer without replacing useful titles for ordinary commands.
    if (!/(?:^|[\s/])(?:ba|z)?sh\s+-[^\s]*c(?:\s|$)/u.test(command)) return;
    let active = true;
    void formatShellForDisplay(command, 88).then((display) => {
      if (!active || !display?.context) return;
      const summary = display.summary || display.text.split("\n").find((line) => line.trim())?.trim();
      if (summary) setHeaderTitle(summary);
    });
    return () => {
      active = false;
    };
  }, [fallbackTitle, item.rawInput, item.toolKind]);
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
          borderRadius: desktop ? `${DESKTOP_INSET_RADIUS}px` : 1,
          // Subtle breathing while a tool is mid-flight; nothing while
          // completed/failed (those are static states).
          animation: running ? `${pulse} 1.6s ease-in-out infinite` : undefined,
        }}
      >
        <Stack
          {...(hasDetail
            ? {
              ...openTap,
              role: "button",
              tabIndex: desktop ? -1 : 0,
              "aria-expanded": selected,
              "aria-haspopup": "dialog" as const,
              onKeyDown: (event: React.KeyboardEvent<HTMLDivElement>): void => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  openDetail();
                }
              },
            }
            : {})}
          {...(hasDetail ? { "data-desktop-item-action": "default" } : {})}
          {...(desktop && hasDetail
            ? {
              "data-desktop-widget-toggle": "tool",
            }
            : {})}
          direction="row"
          spacing={1}
          alignItems="center"
          sx={{
            p: 1,
            cursor: hasDetail ? "pointer" : "default",
            "&:hover": hasDetail ? { bgcolor: "action.hover" } : undefined,
            ...(desktop && hasDetail && {
              outline: "none",
              "&:focus-visible": {
                bgcolor: "action.focus",
                boxShadow: (theme) =>
                  `inset 3px 0 0 ${alpha(theme.palette.primary.main, 0.78)}`,
              },
            }),
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
              fontFamily: item.toolKind === "execute"
                ? "ui-monospace, SFMono-Regular, Menlo, monospace"
                : "var(--cowboy-reading-font, inherit)",
            }}
          >
            {headerTitle}
          </Typography>
          <Chip
            size="small"
            color={toolColor(item.status)}
            label={item.status}
            variant="outlined"
          />
          {hasDetail && (
            <ExpandMore
              fontSize="medium"
              sx={{ transform: "rotate(-90deg)", color: "text.secondary" }}
            />
          )}
        </Stack>
      </Paper>
  );
}

function scrollableAncestor(node: HTMLElement | null): HTMLElement | null {
  let current = node?.parentElement ?? null;
  while (current) {
    const overflow = globalThis.getComputedStyle(current).overflowY;
    if (overflow === "auto" || overflow === "scroll") return current;
    current = current.parentElement;
  }
  return null;
}

type ToolContextBlock = {
  key: string;
  tone: "prose" | "thought" | "user";
  text: string;
};

type FollowingToolPhase = "settled" | "running" | "ready";

const nextToolArrival = keyframes`
  0% { opacity: .68; transform: translateY(3px); }
  100% { opacity: 1; transform: translateY(0); }
`;

function toolContextBlocks(items: RenderItem[]): ToolContextBlock[] {
  const blocks: ToolContextBlock[] = [];
  for (const item of items) {
    let tone: ToolContextBlock["tone"] | null = null;
    let text = "";
    if (item.kind === "message") {
      tone = item.role === "user" ? "user" : "prose";
      text = item.chunks
        .filter((chunk): chunk is Extract<ContentChunk, { type: "text" }> => chunk.type === "text")
        .map((chunk) => chunk.text)
        .join("\n\n");
    } else if (item.kind === "thought") {
      tone = "thought";
      text = item.sections.join("\n\n");
    } else if (item.kind === "lifecycle" && item.detail) {
      tone = "thought";
      text = item.detail;
    }
    if (!tone || !text.trim()) continue;
    const previous = blocks.at(-1);
    if (previous?.tone === tone) {
      previous.text += `\n\n${text}`;
    } else {
      blocks.push({ key: item.key, tone, text });
    }
  }
  return blocks;
}

function ToolTranscriptContext({
  items,
  position,
  phase = "settled",
  defaultOpen = false,
}: {
  items: RenderItem[];
  position: "before" | "after";
  phase?: FollowingToolPhase;
  defaultOpen?: boolean;
}): React.JSX.Element | null {
  const blocks = toolContextBlocks(items);
  const [open, setOpen] = useState(defaultOpen);
  const activeFollowing = position === "after" && phase !== "settled";
  if (blocks.length === 0 && !activeFollowing) return null;
  const label = position === "before" ? "Before this tool" : "After this tool";
  const preview = blocks
    .map((block) =>
      block.text
        .replace(/```[\s\S]*?```/gu, "Code")
        .replace(/[*_~`>#]/gu, "")
        .replaceAll("[", "")
        .replaceAll("]", "")
        .replace(/^[-+]\s+/gu, "")
        .replace(/\s+/gu, " ")
        .trim()
    )
    .filter(Boolean)
    .join(" · ");
  const statusText = phase === "running"
    ? "Working on the next step…"
    : phase === "ready"
    ? "Next tool ready"
    : "";
  const summary = preview || statusText;
  const canExpand = blocks.length > 0;
  return (
    <Box
      aria-label={`${position === "before" ? "Previous" : "Following"} transcript context`}
      sx={{
        border: 1,
        borderColor: open ? "divider" : "transparent",
        bgcolor: open ? "action.hover" : "transparent",
        borderRadius: 1.5,
        overflow: "hidden",
        transition: "background-color .18s ease, border-color .18s ease",
        ...(phase === "ready" && { animation: `${nextToolArrival} .28s cubic-bezier(.2,.8,.2,1) 1` }),
      }}
    >
      <ButtonBase
        aria-expanded={open}
        disabled={!canExpand}
        onClick={(): void => setOpen((value) => !value)}
        sx={{
          width: "100%",
          minHeight: 48,
          px: 1.25,
          py: 0.75,
          display: "grid",
          gridTemplateColumns: "1fr auto",
          columnGap: 1,
          textAlign: "left",
          alignItems: "center",
          borderLeft: 2,
          borderColor: open ? "primary.main" : "divider",
        }}
      >
        <Box sx={{ minWidth: 0 }}>
          <Typography
            variant="overline"
            sx={{ display: "flex", alignItems: "center", gap: 0.625, color: "text.disabled", lineHeight: 1.35 }}
          >
            {label}
            {phase === "running" && <CircularProgress size={10} thickness={5} color="inherit" />}
          </Typography>
          {!open && (
            <Typography
              variant="body2"
              sx={{
                mt: 0.25,
                color: "text.secondary",
                fontWeight: 600,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {summary}
            </Typography>
          )}
        </Box>
        {canExpand && (
          <ExpandMore
            sx={{
              color: "text.secondary",
              transform: open ? "rotate(180deg)" : "none",
              transition: "transform .2s cubic-bezier(.2,.8,.2,1)",
            }}
          />
        )}
      </ButtonBase>
      {open && (
        <Stack spacing={1} sx={{ px: 1.25, pb: 1.25, borderLeft: 2, borderColor: "primary.main" }}>
          {blocks.map((block) => (
            <Box
              key={block.key}
              sx={block.tone === "user"
                ? {
                  bgcolor: "action.selected",
                  borderRadius: 1.25,
                  px: 1,
                  py: 0.5,
                  color: "text.primary",
                }
                : block.tone === "thought"
                ? { fontSize: "0.88em", color: "text.secondary" }
                : { color: "text.primary" }}
            >
              <Markdown text={block.text} />
            </Box>
          ))}
        </Stack>
      )}
    </Box>
  );
}

function ToolDetailsBrowser({
  items,
  runs,
  selectedKey,
  desktop,
  provider,
  onSelect,
  onClose,
  onLocate,
  historyComplete,
}: {
  items: RenderItem[];
  runs: ToolRun[];
  selectedKey: string | null;
  desktop: boolean;
  provider: string;
  onSelect: (key: string) => void;
  onClose: () => void;
  onLocate: (key: string) => void;
  historyComplete: boolean;
}): React.JSX.Element | null {
  const runIndex = selectedKey === null
    ? -1
    : runs.findIndex((candidate) => candidate.tools.some((tool) => tool.key === selectedKey));
  const run = runIndex >= 0 ? runs[runIndex] : undefined;
  const itemIndex = run?.tools.findIndex((tool) => tool.key === selectedKey) ?? -1;
  const item = itemIndex >= 0 ? run?.tools[itemIndex] : undefined;
  const [rawByKey, setRawByKey] = useState<Record<string, boolean>>({});
  const [detailReadyKey, setDetailReadyKey] = useState<string | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const currentRef = useRef<HTMLDivElement>(null);
  const anchorSpacerRef = useRef<HTMLDivElement>(null);
  const historyRef = useRef<HTMLElement>(null);
  const scrollByKey = useRef(new Map<string, number>());
  const navigationChord = useRef<number | null>(null);
  const [navigationPrefixArmed, setNavigationPrefixArmed] = useState(false);
  const rawOnly = item
    ? toolUsesRawOnly({
      kind: item.toolKind,
      toolName: item.toolName,
      title: item.title,
    })
    : false;

  useEffect(() => {
    if (!item) {
      return undefined;
    }
    const id = globalThis.setTimeout(() => setDetailReadyKey(item.key), 320);
    return () => globalThis.clearTimeout(id);
  }, [item?.key]);

  useLayoutEffect(() => {
    if (!item) return;
    const scroller = scrollableAncestor(bodyRef.current);
    const current = currentRef.current;
    const spacer = anchorSpacerRef.current;
    if (!scroller || !current || !spacer) return;
    const saved = scrollByKey.current.get(item.key);
    const align = (): void => {
      // Keep BOTH context directions in normal scroll flow, but add only the
      // trailing room needed for the selected tool to reach the top on a short
      // page. This is layout compensation, not visible content: opening focuses
      // the tool; scrolling upward reveals its preceding prose, downward reveals
      // its following prose. Re-run after the sheet computes its cover height.
      spacer.style.height = "0px";
      const currentTop = scroller.scrollTop +
        current.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
      const maxWithoutSpacer = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      spacer.style.height = `${Math.max(0, currentTop - maxWithoutSpacer + 8)}px`;
      scroller.scrollTop = saved ?? currentTop;
    };
    align();
    const frame = globalThis.requestAnimationFrame(align);
    const settled = globalThis.setTimeout(align, 340);
    return () => {
      globalThis.cancelAnimationFrame(frame);
      globalThis.clearTimeout(settled);
    };
  }, [item?.key]);

  useLayoutEffect(() => {
    if (!desktop || runIndex < 0) return;
    historyRef.current
      ?.querySelector<HTMLElement>(`[data-tool-run-index="${runIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [desktop, runIndex]);

  const select = (next: ToolItem | undefined): void => {
    if (!next || !item) return;
    const scroller = scrollableAncestor(bodyRef.current);
    if (scroller) {
      scrollByKey.current.set(item.key, scroller.scrollTop);
      if (run?.tools.some((tool) => tool.key === next.key)) {
        // A continuous MCP run is one browser page. Selecting one of its rows
        // swaps the page's active call in place: retain the run list's viewport
        // and skip the delayed-detail skeleton. Only cross-run navigation gets
        // the normal focus/anchor alignment performed by the layout effect.
        scrollByKey.current.set(next.key, scroller.scrollTop);
        setDetailReadyKey(next.key);
      }
    }
    onSelect(next.key);
  };

  useEffect(() => {
    if (!desktop || !item) return undefined;
    const clearNavigationChord = (): void => {
      if (navigationChord.current !== null) {
        globalThis.clearTimeout(navigationChord.current);
        navigationChord.current = null;
      }
      setNavigationPrefixArmed(false);
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (desktopImeOwnsKey(event)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) {
        clearNavigationChord();
        return;
      }
      const target = event.target instanceof Element ? event.target : null;
      if (target?.matches("input, textarea, [contenteditable='true']")) return;
      const key = workspaceCommandKey(event);
      if (navigationChord.current !== null) {
        event.preventDefault();
        event.stopPropagation();
        if (event.repeat) return;
        clearNavigationChord();
        if (key === "g") select(runs[0]?.tools[0]);
        return;
      }
      if (key === "Escape") {
        event.preventDefault();
        onClose();
      } else if (key === "[") {
        event.preventDefault();
        select(runs[runIndex - 1]?.tools.at(-1));
      } else if (key === "]") {
        event.preventDefault();
        select(runs[runIndex + 1]?.tools[0]);
      } else if (key === "j") {
        event.preventDefault();
        select(runs[Math.min(runs.length - 1, runIndex + 1)]?.tools[0]);
      } else if (key === "k") {
        event.preventDefault();
        select(runs[Math.max(0, runIndex - 1)]?.tools.at(-1));
      } else if (!rawOnly && (key === "h" || key === "l")) {
        event.preventDefault();
        setRawByKey((state) => ({ ...state, [item.key]: key === "l" }));
      } else if (key === "Enter") {
        event.preventDefault();
        onLocate(item.key);
      } else if (key === "g") {
        event.preventDefault();
        event.stopPropagation();
        if (event.repeat) return;
        setNavigationPrefixArmed(true);
        navigationChord.current = globalThis.setTimeout(() => {
          navigationChord.current = null;
          setNavigationPrefixArmed(false);
        }, 1200);
      } else if (key === "G") {
        event.preventDefault();
        select(runs.at(-1)?.tools.at(-1));
      }
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      clearNavigationChord();
    };
  }, [desktop, runIndex, item, rawOnly, runs, onClose, onLocate]);

  if (!item) return null;
  // A search query has no useful alternate presentation: formatting it only
  // duplicates the query while adding two layers of layout controls. Keep the
  // transport JSON as the single, inspectable representation so input and
  // output retain their exact shape across Codex, Claude and future ACPs.
  const raw = rawOnly || (rawByKey[item.key] ?? false);
  const firstRunItemIndex = items.findIndex((candidate) => candidate.key === run?.tools[0]?.key);
  const lastRunItemIndex = items.findIndex((candidate) => candidate.key === run?.tools.at(-1)?.key);
  const previousToolIndex = runIndex > 0
    ? items.findIndex((candidate) => candidate.key === runs[runIndex - 1]?.tools.at(-1)?.key)
    : -1;
  const nextToolIndex = runIndex < runs.length - 1
    ? items.findIndex((candidate) => candidate.key === runs[runIndex + 1]?.tools[0]?.key)
    : items.length;
  const before = items.slice(previousToolIndex + 1, firstRunItemIndex);
  const after = items.slice(lastRunItemIndex + 1, nextToolIndex);
  const running = item.status === "in_progress" || item.status === "pending";
  const ctx: ToolCtx = {
    provider,
    toolName: item.toolName,
    kind: item.toolKind,
    title: item.title,
    rawInput: item.rawInput && typeof item.rawInput === "object" && !Array.isArray(item.rawInput)
      ? (item.rawInput as Record<string, unknown>)
      : {},
    content: item.content,
    running,
  };
  const heading = toolHeading({
    provider,
    toolName: item.toolName,
    kind: item.toolKind,
    title: item.title,
    rawInput: item.rawInput,
  });
  const runTitle = run?.server
    ? run.server.split(/[-_]/u).filter(Boolean).map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(" ")
    : null;

  const mobileNavigationItems = (
    <>
      <IconButton
        aria-label="Previous tool run"
        disabled={runIndex === 0}
        onClick={(): void => select(runs[runIndex - 1]?.tools.at(-1))}
        sx={{ width: 44, height: 44, justifySelf: "start" }}
      >
        <NavigateBefore />
      </IconButton>
      <Typography
        aria-label={`Tool run ${runIndex + 1} of ${runs.length}${historyComplete ? "" : " loaded"}`}
        variant="caption"
        sx={{ px: 0.75, fontWeight: 700, color: "text.secondary", fontVariantNumeric: "tabular-nums" }}
      >
        {runIndex + 1} / {runs.length}{historyComplete ? "" : "+"}
      </Typography>
      <Tooltip title="Close details">
        <IconButton aria-label="Close tool details" onClick={onClose} sx={{ width: 44, height: 44, justifySelf: "center" }}>
          <Close />
        </IconButton>
      </Tooltip>
      <IconButton
        aria-label="Next tool run"
        disabled={runIndex >= runs.length - 1}
        onClick={(): void => select(runs[runIndex + 1]?.tools[0])}
        sx={{ width: 44, height: 44, justifySelf: "end" }}
      >
        <NavigateNext />
      </IconButton>
      <Tooltip title="Locate in transcript">
        <IconButton aria-label="Locate tool in transcript" onClick={(): void => onLocate(item.key)} sx={{ width: 44, height: 44 }}>
          <MyLocation fontSize="small" />
        </IconButton>
      </Tooltip>
    </>
  );

  const navigation = (
    <Box sx={{ width: "min(100%, 282px)", mx: "auto" }}>
      <FloatingActionIsland columns="44px minmax(58px, 1fr) 44px 44px 44px">
        {mobileNavigationItems}
      </FloatingActionIsland>
    </Box>
  );

  const details = (
      <Box ref={bodyRef} sx={{ position: "relative", maxWidth: desktop ? 1280 : "none", mx: desktop ? "auto" : 0 }}>
        <Stack spacing={1.25}>
            <ToolTranscriptContext
              key={`${item.key}-before`}
              items={before}
              position="before"
              defaultOpen={desktop}
            />
            {run && run.tools.length > 1 && (
              <Box
                aria-label={`${runTitle ?? "MCP"} run with ${run.tools.length} calls`}
                sx={{
                  border: 1,
                  borderColor: "divider",
                  borderRadius: 1.5,
                  overflow: "hidden",
                  bgcolor: "action.hover",
                }}
              >
                <Stack
                  direction="row"
                  alignItems="baseline"
                  justifyContent="space-between"
                  sx={{ px: 1.25, pt: 0.875, pb: 0.5 }}
                >
                  <Typography variant="caption" sx={{ color: "text.secondary", fontWeight: 700 }}>
                    {runTitle}
                  </Typography>
                  <Typography variant="caption" sx={{ color: "text.disabled", fontVariantNumeric: "tabular-nums" }}>
                    {itemIndex + 1} / {run.tools.length} calls
                  </Typography>
                </Stack>
                <Stack sx={{ pb: 0.375 }}>
                  {run.tools.map((tool, callIndex) => {
                    const selected = tool.key === item.key;
                    const callHeading = toolHeading({
                      provider,
                      toolName: tool.toolName,
                      kind: tool.toolKind,
                      title: tool.title,
                      rawInput: tool.rawInput,
                    });
                    return (
                      <ButtonBase
                        key={tool.key}
                        aria-current={selected ? "step" : undefined}
                        onClick={(): void => select(tool)}
                        sx={{
                          width: "100%",
                          minHeight: 42,
                          px: 1.25,
                          display: "grid",
                          gridTemplateColumns: "20px minmax(0, 1fr) auto",
                          gap: 0.75,
                          alignItems: "center",
                          textAlign: "left",
                          bgcolor: selected ? "action.selected" : "transparent",
                          borderLeft: 2,
                          borderColor: selected ? "primary.main" : "transparent",
                        }}
                      >
                        <Typography variant="caption" sx={{ color: selected ? "primary.main" : "text.disabled", fontWeight: 700 }}>
                          {callIndex + 1}
                        </Typography>
                        <Typography variant="body2" noWrap sx={{ fontWeight: selected ? 700 : 500 }}>
                          {callHeading}
                        </Typography>
                        <Typography
                          variant="caption"
                          sx={{
                            color: tool.status === "failed"
                              ? "error.main"
                              : tool.status === "in_progress" || tool.status === "pending"
                              ? "warning.main"
                              : "text.disabled",
                          }}
                        >
                          {tool.status === "in_progress" ? "running" : tool.status}
                        </Typography>
                      </ButtonBase>
                    );
                  })}
                </Stack>
              </Box>
            )}
            <Stack ref={currentRef} direction="row" spacing={1} alignItems="center">
              <Box sx={{ pt: 0.25, color: "text.secondary" }}>
                {toolIcon(item.toolKind)}
              </Box>
              <Typography
                sx={{
                  flex: 1,
                  minWidth: 0,
                  overflowWrap: "anywhere",
                  fontFamily: item.toolKind === "execute"
                    ? "ui-monospace, SFMono-Regular, Menlo, monospace"
                    : "var(--cowboy-reading-font, inherit)",
                }}
              >
                {heading}
              </Typography>
              <Chip
                size="small"
                color={toolColor(item.status)}
                label={item.status}
                variant="outlined"
                sx={{ flexShrink: 0 }}
              />
            </Stack>
            <Box sx={{ borderTop: 1, borderColor: "divider", pt: 1 }}>
              {!rawOnly && (
                <Stack direction="row" alignItems="center" justifyContent="space-between" sx={{ mb: 0.75 }}>
                  <Typography variant="caption" color="text.disabled">
                    {toolVariantLabel({
                      provider,
                      toolName: item.toolName,
                      kind: item.toolKind,
                      title: item.title,
                      rawInput: item.rawInput,
                    })}
                  </Typography>
                  <Stack direction="row" alignItems="center" spacing={0.25}>
                    <Box
                      role="group"
                      aria-label="Tool detail view"
                      sx={{ display: "flex", p: 0.25, borderRadius: 1.25, bgcolor: "action.hover" }}
                    >
                      {[false, true].map((isRaw) => (
                        <ButtonBase
                          key={String(isRaw)}
                          aria-pressed={raw === isRaw}
                          onClick={(): void => setRawByKey((state) => ({ ...state, [item.key]: isRaw }))}
                          sx={{
                            minHeight: 32,
                            px: 1,
                            borderRadius: 1,
                            fontSize: "0.6875rem",
                            fontWeight: raw === isRaw ? 700 : 500,
                            color: raw === isRaw ? "text.primary" : "text.disabled",
                            bgcolor: raw === isRaw ? "background.paper" : "transparent",
                            boxShadow: raw === isRaw ? 1 : 0,
                          }}
                        >
                          {isRaw ? "Raw" : "Formatted"}
                        </ButtonBase>
                      ))}
                    </Box>
                  </Stack>
                </Stack>
              )}
              {detailReadyKey !== item.key
                ? (
                  <Stack spacing={0.75} aria-label="Loading tool details">
                    <Skeleton animation="wave" width="92%" />
                    <Skeleton animation="wave" width="78%" />
                    <Skeleton animation="wave" width="86%" />
                  </Stack>
                )
                : raw
                ? (
                  <Stack spacing={1}>
                    {item.rawInput !== undefined && (
                      <Labeled label="Input">
                        <CodeView
                          code={JSON.stringify(item.rawInput, null, 2) ?? String(item.rawInput)}
                          lang="json"
                          maxHeight={desktop ? 640 : 360}
                          touchWrap={!rawOnly}
                        />
                      </Labeled>
                    )}
                    {item.content !== undefined && (
                      <Labeled label="Output">
                        <CodeView
                          code={JSON.stringify(item.content, null, 2) ?? String(item.content)}
                          lang="json"
                          maxHeight={desktop ? 640 : 360}
                          touchWrap={!rawOnly}
                        />
                      </Labeled>
                    )}
                  </Stack>
                )
                : (
                  <Box>
                    <ToolBody ctx={ctx} />
                  </Box>
                )}
            </Box>
            <ToolTranscriptContext
              key={`${item.key}-after`}
              items={after}
              position="after"
              phase={running ? "running" : runIndex < runs.length - 1 ? "ready" : "settled"}
              defaultOpen={desktop}
            />
            <Box ref={anchorSpacerRef} aria-hidden />
          </Stack>
      </Box>
  );

  const desktopHistory = (
    <Box
      component="nav"
      ref={historyRef}
      aria-label="Tool run history"
      sx={{
        minWidth: 0,
        minHeight: 0,
        overflowY: "auto",
        borderRight: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.default, 0.52),
        userSelect: "none",
      }}
    >
      <Box sx={{ position: "sticky", top: 0, zIndex: 1, px: 1.5, py: 1.25, bgcolor: "background.paper", borderBottom: 1, borderColor: "divider" }}>
        <Typography variant="overline" color="text.secondary" sx={{ fontWeight: 800 }}>
          Agent history
        </Typography>
        <Typography variant="caption" display="block" color="text.disabled">
          {runs.length} runs{historyComplete ? "" : "+"} · newest agent activity last
        </Typography>
      </Box>
      <Stack sx={{ py: 0.5 }}>
        {runs.map((candidate, candidateIndex) => {
          const active = candidateIndex === runIndex;
          const candidateTool = candidate.tools[0];
          if (!candidateTool) return null;
          const candidateHeading = toolHeading({
            provider,
            toolName: candidateTool.toolName,
            kind: candidateTool.toolKind,
            title: candidateTool.title,
            rawInput: candidateTool.rawInput,
          });
          const candidateSummary = toolCopyText({
            title: candidateTool.title,
            toolName: candidateTool.toolName,
            rawInput: candidateTool.rawInput,
          }).split("\n", 1)[0]?.replace(/\s+/g, " ").trim() || candidateHeading;
          return (
            <ButtonBase
              key={candidate.key}
              data-tool-run-index={candidateIndex}
              aria-current={active ? "step" : undefined}
              onClick={(): void => select(candidateTool)}
              sx={{
                minHeight: 50,
                px: 1.5,
                py: 0.75,
                display: "grid",
                gridTemplateColumns: "32px minmax(0, 1fr) auto",
                gap: 1,
                alignItems: "center",
                textAlign: "left",
                borderLeft: 3,
                borderColor: active ? "primary.main" : "transparent",
                bgcolor: active ? "action.selected" : "transparent",
                "&:hover": { bgcolor: "action.hover" },
                "&:focus-visible": {
                  outline: "2px solid",
                  outlineColor: "primary.main",
                  outlineOffset: -2,
                },
              }}
            >
              <Typography variant="caption" color={active ? "primary.main" : "text.disabled"} sx={{ fontWeight: 800, fontVariantNumeric: "tabular-nums" }}>
                {candidateIndex + 1}
              </Typography>
              <Box sx={{ minWidth: 0 }}>
                <Typography variant="body2" noWrap sx={{ fontFamily: candidateTool.toolKind === "execute" ? "ui-monospace, SFMono-Regular, Menlo, monospace" : "inherit", fontWeight: active ? 750 : 550 }}>
                  {candidateSummary}
                </Typography>
                <Typography variant="caption" color="text.disabled" noWrap>
                  {candidateHeading}{candidate.tools.length > 1 ? ` · ${candidate.tools.length} calls` : ""}
                </Typography>
              </Box>
              <Box sx={{ width: 8, height: 8, borderRadius: "50%", bgcolor: candidateTool.status === "failed" ? "error.main" : candidateTool.status === "in_progress" || candidateTool.status === "pending" ? "warning.main" : "success.main" }} />
            </ButtonBase>
          );
        })}
      </Stack>
    </Box>
  );

  return createPortal(
    desktop
      ? (
        <Dialog
          open
          onClose={onClose}
          maxWidth={false}
          fullWidth
          PaperProps={{
            sx: {
              width: "calc(100vw - 32px)",
              maxWidth: 1600,
              height: "calc(100dvh - 32px)",
              maxHeight: 1100,
              m: 2,
              overflow: "hidden",
              borderRadius: 2.5,
              backgroundImage: "none",
            },
          }}
        >
          <Box sx={{ height: "100%", minHeight: 0, display: "grid", gridTemplateRows: "auto minmax(0, 1fr) auto" }}>
            <Box sx={{ px: 2.5, py: 1.25, display: "grid", gridTemplateColumns: "clamp(280px, 24vw, 380px) minmax(0, 1fr)", alignItems: "center", gap: 2.5, borderBottom: 1, borderColor: "divider", userSelect: "none" }}>
              <Box>
                <Typography variant="subtitle1" sx={{ fontWeight: 800 }}>Tool history inspector</Typography>
                <Typography variant="caption" color="text.secondary">
                  Review what the agent ran, changed, and observed
                </Typography>
              </Box>
              <Stack direction="row" alignItems="center" spacing={1.25} sx={{ minWidth: 0 }}>
                <Box sx={{ minWidth: 0, flex: 1 }}>
                  <Typography variant="body2" noWrap sx={{ fontWeight: 750 }}>{heading}</Typography>
                  <Typography variant="caption" color="text.secondary" sx={{ fontVariantNumeric: "tabular-nums" }}>
                    Run {runIndex + 1} of {runs.length}{historyComplete ? "" : "+"}{runTitle ? ` · ${runTitle}` : ""}
                  </Typography>
                </Box>
                <Button
                  disabled={runIndex === 0}
                  onClick={(): void => select(runs[runIndex - 1]?.tools.at(-1))}
                  startIcon={<NavigateBefore />}
                  sx={{ textTransform: "none", flexShrink: 0 }}
                >
                  Previous
                </Button>
                <Button
                  disabled={runIndex >= runs.length - 1}
                  onClick={(): void => select(runs[runIndex + 1]?.tools[0])}
                  endIcon={<NavigateNext />}
                  sx={{ textTransform: "none", flexShrink: 0 }}
                >
                  Next
                </Button>
                <Tooltip title="Locate in transcript">
                  <IconButton aria-label="Locate tool in transcript" onClick={(): void => onLocate(item.key)}>
                    <MyLocation fontSize="small" />
                  </IconButton>
                </Tooltip>
                <Button onClick={onClose} endIcon={<KeyboardArrowDown />} sx={{ textTransform: "none", flexShrink: 0 }}>
                  Close
                </Button>
              </Stack>
            </Box>
            <Box sx={{ minHeight: 0, display: "grid", gridTemplateColumns: "clamp(280px, 24vw, 380px) minmax(0, 1fr)" }}>
              {desktopHistory}
              <DialogContent sx={{ minHeight: 0, overflowY: "auto", px: { md: 3, xl: 4 }, py: 2.5 }}>
                {details}
              </DialogContent>
            </Box>
            <DesktopShortcutBar
                groups={[
                  {
                    label: "Navigate",
                    slots: [
                      {
                        shortcut: "J/K",
                        label: "Run",
                        availability: navigationPrefixArmed ? "inactive" : "available",
                      },
                      {
                        shortcut: "H/L",
                        label: "View",
                        availability: rawOnly || navigationPrefixArmed
                          ? "inactive"
                          : "available",
                      },
                      {
                        shortcut: "Enter",
                        label: "Locate",
                        availability: navigationPrefixArmed ? "inactive" : "available",
                      },
                    ],
                  },
                  {
                    label: "Go",
                    slots: [
                      {
                        shortcut: "G",
                        label: "Prefix",
                        availability: sequentialShortcutAvailability({
                          scopeAvailable: true,
                          armed: navigationPrefixArmed,
                          prefix: true,
                        }),
                      },
                      {
                        shortcut: "G",
                        label: "Oldest",
                        availability: sequentialShortcutAvailability({
                          scopeAvailable: true,
                          armed: navigationPrefixArmed,
                          prefix: false,
                        }),
                      },
                      {
                        shortcut: "Shift+G",
                        label: "Latest",
                        availability: navigationPrefixArmed ? "inactive" : "available",
                      },
                    ],
                  },
                  {
                    slots: [{
                      shortcut: "Esc",
                      label: navigationPrefixArmed ? "Cancel prefix" : "Close",
                    }],
                  },
                ]}
            />
          </Box>
        </Dialog>
      )
      : (
        <Sheet
          open
          onClose={onClose}
          title="Tool details"
          wide
          forceSheet
          cover
          mobileDismiss="none"
          actions={navigation}
        >
          {details}
        </Sheet>
      ),
    document.body,
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
  desktop,
  selectedToolKey,
  onOpenTool,
}: {
  item: RenderItem;
  /** True when this item is the last assistant chunk-bearing item and the
   *  session is still busy. Adds a blinking caret / dots accordingly. */
  streaming?: boolean;
  provider: string;
  desktop: boolean;
  selectedToolKey: string | null;
  onOpenTool: (key: string) => void;
}): React.JSX.Element | null {
  switch (item.kind) {
    case "message":
      return (
        <MessageBubble
          role={item.role}
          chunks={item.chunks}
          streaming={!!streaming && item.role === "assistant"}
          autoResumed={item.autoResumed === true}
          provider={provider}
          desktop={desktop}
        />
      );
    case "thought":
      return (
        <Box
          sx={{
            color: "text.secondary",
            alignSelf: "stretch",
            maxWidth: "100%",
            px: provider === "codex" || provider === "codex-deepseek" ? 0.25 : 0,
          }}
        >
          <Box sx={{ fontSize: "0.84rem", flex: 1, minWidth: 0 }}>
            {/* Empty Codex HTML separators become compact, connected steps. */}
            <ThoughtSteps
              sections={item.sections}
              streaming={!!streaming}
              codex={provider === "codex" || provider === "codex-deepseek"}
              touch={!desktop}
            />
          </Box>
        </Box>
      );
    case "tool":
      return (
        <ToolCard
          item={item}
          desktop={desktop}
          selected={selectedToolKey === item.key}
          onOpen={onOpenTool}
        />
      );
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
          {interrupted
            ? <WarningAmberRounded fontSize="medium" />
            : <ErrorOutline fontSize="medium" />}
          <Typography
            variant="caption"
            sx={{ fontWeight: interrupted ? 600 : 400 }}
          >
            {item.status}
            {item.detail ? `: ${presentedCrashDetail(item.detail)}` : ""}
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
          <Stack
            direction="row"
            spacing={0.5}
            alignItems="center"
            sx={{ flexShrink: 0 }}
          >
            <CleaningServices sx={{ fontSize: "0.95rem" }} />
            <Typography
              variant="caption"
              sx={{ fontWeight: 600, letterSpacing: 0.3 }}
            >
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
const GEMINI_CONSUMER_RETIRED =
  "This client is no longer supported for Gemini Code Assist for individuals";
const GEMINI_CONSUMER_GUIDANCE =
  "Gemini CLI no longer supports Google Login for personal, Google AI Pro, or AI Ultra accounts. Use an API key, Code Assist Standard/Enterprise, or migrate to Antigravity.";

function presentedCrashDetail(detail: string): string {
  return detail.includes(GEMINI_CONSUMER_RETIRED) ? GEMINI_CONSUMER_GUIDANCE : detail;
}

function latestCrashDetail(items: RenderItem[]): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item?.kind === "lifecycle" && item.status === "crashed" && item.detail) {
      return presentedCrashDetail(item.detail);
    }
  }
  return null;
}

function SessionStatusBar({
  status,
  crashDetail,
}: {
  status: Status;
  crashDetail: string | null;
}): React.JSX.Element | null {
  let tone: "warning" | "error" | "neutral";
  let icon: React.JSX.Element;
  let text: string;
  if (status === "interrupted") {
    tone = "warning";
    icon = <WarningAmberRounded fontSize="small" />;
    text =
      "Last turn was interrupted before it finished — send a message to start a new one.";
  } else if (status === "crashed") {
    tone = "error";
    icon = <ErrorOutline fontSize="small" />;
    text = crashDetail ?? "Agent stopped unexpectedly — send a message to restart it.";
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
        const main = tone === "error"
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

// `column-reverse` already keeps a followed transcript at scrollTop 0. Writing
// the same value for every streamed ACP envelope is not a harmless no-op on
// iOS WebKit: it schedules a scroll-layer update while adjacent thought and
// tool rows are still being laid out, so one frame can paint the old card layer
// over the newly positioned thought. Only correct real drift from the live
// edge; leave an already-pinned compositor completely alone.
function pinTranscriptToLatest(el: HTMLElement): void {
  if (Math.abs(el.scrollTop) > 0.5) el.scrollTop = 0;
}

// Snapshot the message row at the viewport CENTRE + its offset from the
// container top. Centre (not the top edge) so the probe always lands on a row,
// never in the container's top padding / status-strip inset.
function captureFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  const r = el.getBoundingClientRect();
  const hit = el.ownerDocument.elementFromPoint(
    r.left + r.width / 2,
    r.top + r.height / 2,
  );
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
  const row = el.querySelector<HTMLElement>(
    `[data-key="${CSS.escape(a.key)}"]`,
  );
  if (!row) return;
  const delta = row.getBoundingClientRect().top -
    el.getBoundingClientRect().top - a.top;
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
function itemProgressSignature(
  item: RenderItem | undefined,
  count: number,
): string {
  if (!item) return "";
  switch (item.kind) {
    case "message":
      return `${count}:${item.key}:m:${
        item.chunks.map((chunk) =>
          chunk.type === "text" ? chunk.text.length : chunk.src.length
        ).join(",")
      }`;
    case "thought":
      return `${count}:${item.key}:t:${
        item.sections.map((section) => section.length).join(",")
      }`;
    case "tool":
      return `${count}:${item.key}:tool:${item.status}:${item.title}`;
    case "permission":
      return `${count}:${item.key}:permission:${item.resolved}:${
        item.chosen ?? ""
      }`;
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
  judging = false,
  topInset,
  bottomInset,
  onScrollableChange,
  desktopNavigation = false,
  historyPaging = "scroll",
  visibleItemKeys,
  liveTail = true,
  shortContentAtTop = false,
  pageFooter,
  pageId,
  restoreAnchorKey,
  onAnchorRestored,
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
  /** Transient post-turn judge work. It belongs to the live Transcript tail;
   * settled/actionable verdicts remain in the Composer status stack. */
  judging?: boolean | undefined;
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
  /** History owns scroll-threshold and viewport-fill pagination. Page View
   * owns question-boundary loading outside this shared renderer, so ordinary
   * scrolling must never mutate its retained event window. */
  historyPaging?: "scroll" | "page";
  /** Explore projection: render only this page's canonical item keys. History
   * omits the prop and retains the established full-timeline path. */
  visibleItemKeys?: ReadonlySet<string> | undefined;
  /** Whether this projection contains the live timeline tail. Old Explore pages
   * must not show the current turn's spinner or streaming caret. */
  liveTail?: boolean | undefined;
  /** Page projections read like documents: when one page is shorter than the
   * viewport, keep it at the visual top. History deliberately remains anchored
   * beside the composer. In the column-reverse scroller a DOM-first flexible
   * spacer occupies the visual bottom without changing overflow behaviour. */
  shortContentAtTop?: boolean | undefined;
  /** Page View-only navigation rendered after the page document. Because the
   * transcript is column-reverse, this slot is DOM-first, before the flexible
   * short-page spacer, so its visual position remains below the final row. */
  pageFooter?: React.ReactNode;
  /** Explore's current question page. A page viewport is restored only when
   * returning to the same session and page; page navigation clears its cache. */
  pageId?: string | undefined;
  /** Projection transition: restore this canonical row near the viewport centre. */
  restoreAnchorKey?: string | null | undefined;
  onAnchorRestored?: (() => void) | undefined;
}): React.JSX.Element {
  const managesScrollHistory = historyPaging === "scroll";
  // Memoized on `timeline` identity: `applyEnvelope` (store.ts) only hands us a
  // new array when a new event actually lands, so this O(n) fold runs once per
  // event — NOT on every scroll-driven re-render. Stable item identities also
  // let the `memo`'d `ItemView` rows skip re-rendering.
  // Keep receiving canonical envelopes while native scrolling owns the viewport,
  // but present one stable snapshot until momentum/smooth scrolling settles.
  // This removes Markdown/layout work from WebKit's scrolling frames without
  // dropping or delaying transport data; the latest timeline flushes atomically.
  const [renderPausedForScroll, setRenderPausedForScroll] = useState(false);
  const presentedTimelineRef = useRef(timeline);
  const latestTimelineRef = useRef(timeline);
  latestTimelineRef.current = timeline;
  const drawerCatchupActiveRef = useRef(false);
  const [drawerCatchupStep, setDrawerCatchupStep] = useState(0);
  if (!renderPausedForScroll && !drawerCatchupActiveRef.current) {
    presentedTimelineRef.current = timeline;
  } else if (managesScrollHistory) {
    // Native momentum keeps live tail changes frozen, but an older history page
    // is safe to reveal immediately: column-reverse inserts it above the current
    // viewport and preserves every already-visible envelope by reference. Do
    // not make a 10ms page wait for a long trackpad/touch settle interval.
    presentedTimelineRef.current = revealHistoryPrepend(
      presentedTimelineRef.current,
      timeline,
    );
  }
  const presentedTimeline = presentedTimelineRef.current;
  const startDrawerCatchupRef = useRef<() => void>(() => undefined);
  startDrawerCatchupRef.current = (): void => {
    drawerCatchupActiveRef.current = true;
    startTransition(() => setDrawerCatchupStep((step) => step + 1));
  };
  useEffect(() => {
    if (!drawerCatchupActiveRef.current) return undefined;
    const frame = requestAnimationFrame(() => {
      if (!drawerCatchupActiveRef.current) return;
      const next = advanceTimelinePresentation(
        presentedTimelineRef.current,
        canonicalTimeline(sessionId) ?? latestTimelineRef.current,
        // Keep each Markdown/DOM reconciliation below one display frame while
        // still catching ordinary streamed prose faster than it arrives.
        640,
        2,
      );
      presentedTimelineRef.current = next.timeline;
      if (next.complete) {
        drawerCatchupActiveRef.current = false;
        startTransition(() => setRenderPausedForScroll(false));
        return;
      }
      startTransition(() => setDrawerCatchupStep((step) => step + 1));
    });
    return () => cancelAnimationFrame(frame);
  }, [drawerCatchupStep, sessionId]);
  const allItems = useMemo(() => derive(presentedTimeline), [presentedTimeline]);
  const crashDetail = useMemo(() => latestCrashDetail(allItems), [allItems]);
  const items = useMemo(
    () =>
      visibleItemKeys
        ? allItems.filter((item) => visibleItemKeys.has(item.key))
        : allItems,
    [allItems, visibleItemKeys],
  );
  const runs = useMemo(() => toolRuns(items), [items]);
  const tools = useMemo(() => runs.flatMap((run) => run.tools), [runs]);
  const [selectedToolKey, setSelectedToolKey] = useState<string | null>(null);
  const [locatedToolKey, setLocatedToolKey] = useState<string | null>(null);
  const locateTimerRef = useRef<number | null>(null);
  const openTool = useCallback((key: string): void => setSelectedToolKey(key), []);
  const closeTool = useCallback((): void => setSelectedToolKey(null), []);

  useEffect(() => {
    if (selectedToolKey !== null && !tools.some((tool) => tool.key === selectedToolKey)) {
      setSelectedToolKey(null);
    }
  }, [selectedToolKey, tools]);

  useEffect(() => () => {
    if (locateTimerRef.current !== null) globalThis.clearTimeout(locateTimerRef.current);
  }, []);
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
  const working = connected && busy && liveTail;
  // The judge is launched only after Busy settles back to Running. Requiring
  // that authoritative state prevents a stale flag from surviving a crash,
  // interruption, reconnect, or immediately-started next turn as a false live
  // activity row.
  const showJudging = judging && connected && liveTail && status === "running";
  // No messages yet + a LIVE session (a freshly created session is Running-idle,
  // waiting for the first prompt) → show the "send a message to start" empty state
  // instead of a blank wall. Non-live empties (exited/interrupted/crashed) are
  // already covered by SessionStatusBar's hint, so they fall through to a plain
  // empty area + that bar (no duplicate).
  const isLive = status === "starting" || status === "running" ||
    status === "busy";
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
  const lastIsStreamingAssistant = working &&
    !!lastItem &&
    ((lastItem.kind === "message" && lastItem.role === "assistant") ||
      lastItem.kind === "thought");
  const compacting = working && isCompactingTail(timeline);
  const compactedAtTail = isCompactionCompletionTail(timeline);
  // Show the trailing "dots" row when working AND we're NOT already showing
  // a caret-tipped streaming assistant bubble at the bottom (i.e. between
  // sending a prompt and the first chunk landing, or after a tool call
  // completes while waiting for the model to start text again).
  const showTrailingDots = working && !lastIsStreamingAssistant &&
    !compacting && !compactedAtTail;
  const parentRef = useRef<HTMLDivElement>(null);
  const managesScrollHistoryRef = useRef(managesScrollHistory);
  managesScrollHistoryRef.current = managesScrollHistory;
  // Switching from Explore remounts the History transcript. Remember whether
  // that mount carries a viewport hand-off so the ordinary "new session starts
  // at latest" initializer below cannot overwrite the restored reading point.
  const restoringProjectionMountRef = useRef(Boolean(restoreAnchorKey));
  useLayoutEffect(() => {
    if (!restoreAnchorKey) return;
    const row = parentRef.current?.querySelector<HTMLElement>(
      `[data-key="${CSS.escape(restoreAnchorKey)}"]`,
    );
    if (!row) return;
    stick.current = false;
    setSticky(sessionId, false);
    row.scrollIntoView({ block: "center", behavior: "auto" });
    onAnchorRestored?.();
  }, [items, onAnchorRestored, restoreAnchorKey, sessionId]);
  // Report scroll-overflow (content taller than the viewport) to the parent so
  // the composer slab can gate its up-shadow on real scrollable content. Kept in
  // refs so the once-wired scroll effect calls the latest callback without
  // re-binding, and only fires on a CHANGE (cheap to call every scroll/chunk).
  const onScrollableChangeRef = useRef(onScrollableChange);
  onScrollableChangeRef.current = onScrollableChange;
  const lastScrollableRef = useRef<boolean | null>(null);
  const reportScrollableRef = useRef<() => void>(() => undefined);
  const viewportBackfillRafRef = useRef(0);
  const viewportBackfillSettleTimerRef = useRef<number | null>(null);
  const viewportBackfillSettlingRef = useRef(false);
  const viewportBackfillCursorRef = useRef<number | null>(null);
  const viewportBackfillAllowanceRef = useRef(
    VIEWPORT_BACKFILL_PAGE_LIMIT,
  );
  const viewportHeightRef = useRef<number | null>(null);
  const historyPrefetchArmedRef = useRef(true);
  const scrollbackRetryCountRef = useRef(0);
  const scrollbackRetryTimerRef = useRef<number | null>(null);
  const scrollbackFillActiveRef = useRef(false);
  const scrollbackFillRunRef = useRef(0);
  const requestOlderPageRef = useRef<() => void>(() => undefined);
  const requestViewportBackfillRef = useRef<
    (fromResize: boolean) => void
  >(() => undefined);
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
  // Native scrolling temporarily owns scrollTop on both products. Streaming
  // chunks must not replay the detached anchor while touch momentum, wheel
  // inertia, keyboard scrolling, or a scrollbar drag is still moving the
  // viewport; doing so makes the two writers alternate and presents as a stuck
  // or flashing transcript.
  const nativeScrollActiveRef = useRef(false);
  // History pagination state for this session (from the store): drives the
  // "loading older…" indicator at the top + the reached-start cutoff.
  const paging = useStoreSelector((snapshot) =>
    snapshot.pagination.get(sessionId)
  );
  const pagingRef = useRef(paging);
  pagingRef.current = paging;
  const renderableItemCountRef = useRef(items.length);
  renderableItemCountRef.current = items.length;
  const timelineEventCountRef = useRef(timeline.length);
  timelineEventCountRef.current = timeline.length;
  const semanticRecoveryCursorRef = useRef<number | null>(null);
  const showFreshSessionEmptyState = shouldShowFreshSessionEmptyState({
    loading,
    itemCount: items.length,
    isLive,
    reachedStart: paging?.reachedStart,
    timelineEventCount: timeline.length,
  });

  // Tool details deliberately browses only the retained history window. Never
  // page an entire long-running session merely to make its denominator exact:
  // thousands of newly derived transcript rows behind the sheet turn an
  // otherwise local expand/collapse into a main-thread layout storm. The
  // footer keeps its `+` suffix until ordinary transcript scrollback happens
  // to reach the beginning, so the bounded count stays honest without doing
  // hidden work.
  // A retained recent tail can arrive before enough older pages have filled a
  // tall phone viewport. Without an explicit state the latest reply sits above
  // the composer while the unused upper viewport looks like a broken blank
  // screen. Keep this distinct from ordinary scrollback loading: it is only the
  // automatic, followed-mode viewport bootstrap below.
  const [backfillingViewport, setBackfillingViewport] = useState(false);
  const [viewportBackfillPaused, setViewportBackfillPaused] = useState(false);
  const [scrollbackLoading, setScrollbackLoading] = useState(false);
  const [scrollbackFailed, setScrollbackFailed] = useState(false);
  const [scrollbackFillHeight, setScrollbackFillHeight] = useState(0);
  const [showHistoryLoadingFill, setShowHistoryLoadingFill] = useState(false);
  useEffect(() => {
    if (!backfillingViewport) {
      setShowHistoryLoadingFill(false);
      return undefined;
    }
    // Only the initial viewport bootstrap reserves the otherwise-empty reading
    // area. User-initiated scrollback uses the geometry-neutral overlay below.
    setShowHistoryLoadingFill(true);
    return undefined;
  }, [backfillingViewport]);
  requestOlderPageRef.current = (): void => {
    const el = parentRef.current;
    if (
      !managesScrollHistoryRef.current || !el ||
      scrollbackFillActiveRef.current
    ) return;
    const run = ++scrollbackFillRunRef.current;
    scrollbackFillActiveRef.current = true;
    setScrollbackFailed(false);
    const targetHeight = Math.min(240, Math.max(144, el.clientHeight * 0.24));
    const idleBandHeight = el.querySelector<HTMLElement>(
      "[data-transcript-scrollback-fill]",
    )?.getBoundingClientRect().height ?? 0;
    const baseScrollHeight = Math.max(0, el.scrollHeight - idleBandHeight);
    const baseRowCount = el.querySelectorAll<HTMLElement>("[data-key]").length;
    // The boundary already exists at the visual page head. Promote it to its
    // active height before starting I/O so loaded rows replace a visible
    // placeholder instead of making a late spinner flash above them.
    setScrollbackLoading(true);
    setScrollbackFillHeight(targetHeight);
    void (async (): Promise<void> => {
      let mountedContent = false;
      try {
        // Give React/WebKit one paint with the promoted boundary before the
        // first network request can resolve and prepend rows.
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
        if (scrollbackFillRunRef.current !== run) return;
        for (let page = 0; page < SCROLLBACK_FILL_PAGE_LIMIT; page += 1) {
          const previousBandHeight = el.querySelector<HTMLElement>(
            "[data-transcript-scrollback-fill]",
          )?.getBoundingClientRect().height ?? 0;
          const previousRowCount = el.querySelectorAll<HTMLElement>("[data-key]").length;
          const previousContentHeight = Math.max(
            0,
            el.scrollHeight - previousBandHeight,
          );
          const progressed = await loadOlder(sessionIdRef.current);
          if (!progressed) {
            const currentPaging = pagingRef.current;
            if (
              currentPaging && !currentPaging.reachedStart &&
              currentPaging.beforeSeq !== null &&
              scrollbackRetryCountRef.current < 1
            ) {
              scrollbackRetryCountRef.current += 1;
              scrollbackRetryTimerRef.current = globalThis.setTimeout(() => {
                scrollbackRetryTimerRef.current = null;
                const currentEl = parentRef.current;
                if (!currentEl) return;
                const fromBottom = Math.abs(currentEl.scrollTop);
                const fromTop = currentEl.scrollHeight - currentEl.clientHeight - fromBottom;
                if (fromTop <= currentEl.clientHeight * 3) {
                  requestOlderPageRef.current();
                }
              }, 700);
            } else if (currentPaging && !currentPaging.reachedStart) {
              setScrollbackFailed(true);
            }
            break;
          }
          scrollbackRetryCountRef.current = 0;
          // The store publishes as soon as HTTP completes, but iPhone render
          // pacing can defer the corresponding React rows for several frames.
          // Measure only after a real row or content-height change reaches DOM;
          // otherwise the fetched page is mistaken for zero replacement.
          await waitForScrollbackMount(
            el,
            previousRowCount,
            previousContentHeight,
            () => scrollbackFillRunRef.current === run,
          );
          const mountedRowCount = el.querySelectorAll<HTMLElement>("[data-key]")
            .length;
          const mountedBandHeight = el.querySelector<HTMLElement>(
            "[data-transcript-scrollback-fill]",
          )?.getBoundingClientRect().height ?? 0;
          const mountedContentHeight = Math.max(0, el.scrollHeight - mountedBandHeight);
          mountedContent = mountedContent ||
            mountedRowCount > previousRowCount ||
            mountedContentHeight > previousContentHeight + 1;
          await new Promise<void>((resolve) => {
            globalThis.setTimeout(() => requestAnimationFrame(() => resolve()),
              SCROLLBACK_FILL_SETTLE_MS);
          });
          if (scrollbackFillRunRef.current !== run) return;
          const bandHeight = el.querySelector<HTMLElement>(
            "[data-transcript-scrollback-fill]",
          )?.getBoundingClientRect().height ?? 0;
          const remaining = scrollbackFillRemaining({
            targetHeight,
            baseScrollHeight,
            currentScrollHeight: el.scrollHeight,
            skeletonHeight: bandHeight,
          });
          // Real rows consume the visible placeholder from its lower edge. A
          // remaining cursor keeps the next quiet skeleton group at page head.
          setScrollbackFillHeight(
            Math.max(SCROLLBACK_IDLE_BOUNDARY_HEIGHT, remaining),
          );
          const currentPaging = pagingRef.current;
          const loadedRows = Math.max(
            0,
            el.querySelectorAll<HTMLElement>("[data-key]").length - baseRowCount,
          );
          const fromBottom = Math.abs(el.scrollTop);
          const fromTop = el.scrollHeight - el.clientHeight - fromBottom;
          if (!currentPaging || !shouldContinueScrollbackFill({
            remaining,
            loadedRows,
            minimumRows: SCROLLBACK_FILL_MINIMUM_ROWS,
            fromTop,
            viewportHeight: el.clientHeight,
            reachedStart: currentPaging.reachedStart,
            // `loadOlder` has completed canonically; React may not have
            // published that synchronous store update into this ref yet.
            // Treating its stale `loadingOlder` bit as authoritative would
            // stop sparse-page filling after the first request.
            loadingOlder: false,
            beforeSeq: currentPaging.beforeSeq,
          })) break;
        }
      } finally {
        if (scrollbackFillRunRef.current === run) {
          scrollbackFillActiveRef.current = false;
          setScrollbackLoading(false);
          setScrollbackFillHeight(SCROLLBACK_IDLE_BOUNDARY_HEIGHT);
          // State above changes the active placeholder into the quiet boundary
          // for the *next* page. Wait until that exact geometry is painted,
          // then move only a reader who is still inside the boundary past it.
          // This is the missing replacement hand-off: fetched rows take the
          // old skeleton's place while the next skeleton remains just above.
          if (mountedContent) {
            requestAnimationFrame(() => requestAnimationFrame(() => {
              if (scrollbackFillRunRef.current !== run) return;
              const currentEl = parentRef.current;
              const boundary = currentEl?.querySelector<HTMLElement>(
                "[data-transcript-scrollback-fill]",
              );
              if (!currentEl || !boundary) return;
              const fromBottom = Math.abs(currentEl.scrollTop);
              const currentFromTop = Math.max(
                0,
                currentEl.scrollHeight - currentEl.clientHeight - fromBottom,
              );
              const desiredFromTop = scrollbackReplacementFromTop({
                currentFromTop,
                boundaryHeight: boundary.getBoundingClientRect().height,
                mountedContent: true,
              });
              if (desiredFromTop === null) return;
              const desiredFromBottom = Math.max(
                0,
                currentEl.scrollHeight - currentEl.clientHeight - desiredFromTop,
              );
              const direction = currentEl.scrollTop > 0.5 ? 1 : -1;
              currentEl.scrollTop = direction * desiredFromBottom;
            }));
          }
        }
      }
    })();
  };
  useEffect(() => {
    setBackfillingViewport(false);
    setScrollbackLoading(false);
    setScrollbackFailed(false);
    setScrollbackFillHeight(0);
    setShowHistoryLoadingFill(false);
    setViewportBackfillPaused(false);
    viewportBackfillCursorRef.current = null;
    viewportBackfillSettlingRef.current = false;
    semanticRecoveryCursorRef.current = null;
    viewportBackfillAllowanceRef.current = VIEWPORT_BACKFILL_PAGE_LIMIT;
    viewportHeightRef.current = null;
    historyPrefetchArmedRef.current = true;
    scrollbackRetryCountRef.current = 0;
    scrollbackFillRunRef.current += 1;
    scrollbackFillActiveRef.current = false;
    requestViewportBackfillRef.current(false);
    return () => {
      scrollbackFillRunRef.current += 1;
      scrollbackFillActiveRef.current = false;
      if (scrollbackRetryTimerRef.current !== null) {
        globalThis.clearTimeout(scrollbackRetryTimerRef.current);
        scrollbackRetryTimerRef.current = null;
      }
      if (viewportBackfillSettleTimerRef.current !== null) {
        globalThis.clearTimeout(viewportBackfillSettleTimerRef.current);
        viewportBackfillSettleTimerRef.current = null;
      }
      viewportBackfillSettlingRef.current = false;
      if (viewportBackfillRafRef.current !== 0) {
        cancelAnimationFrame(viewportBackfillRafRef.current);
        viewportBackfillRafRef.current = 0;
      }
    };
  }, [sessionId]);

  // Hydrate one immutable history page at a time. The flex skeleton is both the
  // visual placeholder and the feedback signal: its measured height is exactly
  // the still-unused reading area. Each page is rendered and allowed to settle
  // before deciding whether another is useful, so ordinary conversations fetch
  // very little while sparse generated-image histories converge without blanks.
  requestViewportBackfillRef.current = (fromResize: boolean): void => {
    if (!managesScrollHistoryRef.current) {
      setBackfillingViewport(false);
      return;
    }
    if (viewportBackfillRafRef.current !== 0) return;
    viewportBackfillRafRef.current = requestAnimationFrame(() => {
      viewportBackfillRafRef.current = 0;
      const el = parentRef.current;
      if (!el || !paging) {
        setBackfillingViewport(false);
        return;
      }
      // A bounded tail can consist entirely of output deltas whose original
      // tool_call is much older. Walking 64 raw rows at a time is both slow and
      // incapable of producing UI until that parent finally arrives. Jump to
      // the indexed previous question page instead; the server bounds it at
      // TurnEnd, so background terminal output cannot make this recovery
      // response grow forever.
      if (shouldRecoverUnrenderableHistory({
        managed: managesScrollHistoryRef.current,
        itemCount: renderableItemCountRef.current,
        timelineEventCount: timelineEventCountRef.current,
        reachedStart: paging.reachedStart,
        loadingOlder: paging.loadingOlder,
        beforeSeq: paging.beforeSeq,
      })) {
        const cursor = paging.beforeSeq;
        if (semanticRecoveryCursorRef.current === cursor) {
          setBackfillingViewport(false);
          return;
        }
        semanticRecoveryCursorRef.current = cursor;
        setBackfillingViewport(true);
        void loadPreviousQuestionPage(sessionId)
          .then((progressed) => {
            if (!progressed) setScrollbackFailed(true);
          })
          .finally(() => {
            requestViewportBackfillRef.current(false);
          });
        return;
      }
      if (viewportHeightRef.current === null) {
        viewportHeightRef.current = el.clientHeight;
      }
      const loadingFill = el.querySelector<HTMLElement>(
        "[data-transcript-loading-fill]",
      );
      const loadingFillHeight = loadingFill?.getBoundingClientRect().height ?? null;
      const hasVisibleGap = shouldBackfillTranscriptViewport({
        managed: managesScrollHistoryRef.current,
        allowed: true,
        desktop: desktopNavigation,
        fromResize,
        reachedStart: paging.reachedStart,
        loadingOlder: false,
        beforeSeq: paging.beforeSeq,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
        loadingFillHeight,
      });
      const needsOlderPage = shouldBackfillTranscriptViewport({
        managed: managesScrollHistoryRef.current,
        allowed: viewportBackfillAllowanceRef.current > 0 &&
          !viewportBackfillSettlingRef.current,
        desktop: desktopNavigation,
        fromResize,
        reachedStart: paging.reachedStart,
        loadingOlder: paging.loadingOlder,
        beforeSeq: paging.beforeSeq,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
        loadingFillHeight,
      });
      const requestOwned = viewportBackfillCursorRef.current !== null ||
        paging.loadingOlder || viewportBackfillSettlingRef.current;
      setBackfillingViewport(hasVisibleGap);
      setViewportBackfillPaused(
        hasVisibleGap && !requestOwned &&
          viewportBackfillAllowanceRef.current <= 0,
      );
      if (
        needsOlderPage &&
        paging.beforeSeq !== viewportBackfillCursorRef.current
      ) {
        const requestedCursor = paging.beforeSeq;
        viewportBackfillAllowanceRef.current -= 1;
        viewportBackfillSettlingRef.current = true;
        viewportBackfillCursorRef.current = requestedCursor;
        void loadOlder(sessionId).finally(() => {
          if (viewportBackfillCursorRef.current === requestedCursor) {
            viewportBackfillCursorRef.current = null;
            // Let the new rows, images and Markdown establish real geometry
            // before spending another page. Later image decode/fallback changes
            // are covered by the row ResizeObserver below.
            if (viewportBackfillSettleTimerRef.current !== null) {
              globalThis.clearTimeout(viewportBackfillSettleTimerRef.current);
            }
            viewportBackfillSettleTimerRef.current = globalThis.setTimeout(() => {
              viewportBackfillSettleTimerRef.current = null;
              viewportBackfillSettlingRef.current = false;
              requestViewportBackfillRef.current(false);
            }, VIEWPORT_BACKFILL_SETTLE_MS);
          }
        });
      }
    });
  };

  // Re-check after a page lands so a still-short viewport can spend the next
  // bounded allowance. Real overflow or reachedStart stops the chain.
  useLayoutEffect(() => {
    requestViewportBackfillRef.current(false);
  }, [
    items.length,
    paging?.beforeSeq,
    paging?.loadingOlder,
    paging?.reachedStart,
    sessionId,
  ]);
  // Large generated images and rich Markdown can initially reserve enough
  // height to stop viewport hydration, then shrink after decode/fallback or a
  // disclosure settles. Observing only the fixed-height scroll box misses that
  // change because its border box never resized. Watch the retained rows too,
  // and re-measure through the existing RAF-coalesced loader whenever their
  // actual layout changes. Filled/desktop/page-view guards remain centralized
  // in requestViewportBackfillRef, so streaming rows do not create extra loads.
  useEffect(() => {
    if (!managesScrollHistory || typeof ResizeObserver === "undefined") {
      return undefined;
    }
    const el = parentRef.current;
    if (!el) return undefined;
    const observer = new ResizeObserver(() => {
      requestViewportBackfillRef.current(false);
    });
    for (const row of el.querySelectorAll<HTMLElement>("[data-key]")) {
      observer.observe(row);
    }
    return () => observer.disconnect();
  }, [items.length, managesScrollHistory, sessionId]);
  // This device's optimistic chat sends awaiting the daemon echo — rendered as
  // user bubbles below the latest real item (newest at the very bottom), dropped
  // by cmid the moment the echo lands. Empty in the common (confirmed) case.
  const optimisticMsgs = useStoreSelector(
    (snapshot) =>
      snapshot.optimisticMessages.get(sessionId) ?? EMPTY_OPTIMISTIC_MESSAGES,
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
  //   confuse us. Bottom-bound wheel/key repeat is ignored so its tail cannot
  //   undo automatic re-enable at the boundary.
  // - History re-enables stick inside the bottom magnetic zone. Page View does
  //   so only for the live page while it is actively streaming; a settled
  //   page's button and scrolling remain plain navigation.
  // - Auto-snap on new items uses the **`stick` value at the time of the
  //   commit** — i.e. via a ref so we don't re-render to keep it.
  // - The on/off state is mirrored to the per-session stickyStore so the
  //   composer's sticky toggle reflects + drives it (it shows active when
  //   stuck, and a tap bumps scrollNonce → we scroll to the bottom below).
  const stick = useRef(true);
  const workingRef = useRef(working);
  workingRef.current = working;
  // Transcript is NOT remounted per session (it re-pins via the sessionId
  // effect), so the once-wired scroll listeners would capture a stale
  // sessionId. Read it through a ref that tracks the latest prop.
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  const pageIdRef = useRef(pageId);
  pageIdRef.current = pageId;
  const viewportRestoreActiveRef = useRef(false);
  const pageWorkingRef = useRef({
    key: `${sessionId}:${pageId ?? ""}`,
    working,
  });
  const locateTool = useCallback((key: string): void => {
    setSelectedToolKey(null);
    stick.current = false;
    setSticky(sessionIdRef.current, false);
    setLocatedToolKey(key);
    if (locateTimerRef.current !== null) globalThis.clearTimeout(locateTimerRef.current);
    // Let the cover sheet unmount before scrolling the transcript beneath it.
    globalThis.setTimeout(() => {
      const row = [...(parentRef.current?.querySelectorAll<HTMLElement>("[data-key]") ?? [])]
        .find((candidate) => candidate.dataset["key"] === key);
      // Tool history can be tens of thousands of pixels away. An animated
      // journey delays the destination highlight until after it has already
      // faded; jump atomically, then let the 1.4s highlight orient the reader.
      row?.scrollIntoView({ block: "center", behavior: "auto" });
      locateTimerRef.current = globalThis.setTimeout(() => {
        setLocatedToolKey((current) => current === key ? null : current);
        locateTimerRef.current = null;
      }, 1400);
    }, 40);
  }, []);
  // Bumped by the composer toggle (requestStickToBottom) to ask us to scroll to
  // the bottom now; the effect below reacts to a change.
  const scrollNonce = useScrollNonce(sessionId);
  const historyReleaseTimerRef = useRef<number | undefined>(undefined);
  const cancelHistoryRelease = (): void => {
    if (historyReleaseTimerRef.current === undefined) return;
    globalThis.clearTimeout(historyReleaseTimerRef.current);
    historyReleaseTimerRef.current = undefined;
  };
  const scheduleHistoryRelease = (): void => {
    cancelHistoryRelease();
    if (!managesScrollHistoryRef.current) return;
    historyReleaseTimerRef.current = globalThis.setTimeout(() => {
      historyReleaseTimerRef.current = undefined;
      const el = parentRef.current;
      if (
        !stick.current || nativeScrollActiveRef.current || !el ||
        Math.abs(el.scrollTop) > 1 || el.scrollHeight <= el.clientHeight + 1
      ) return;
      releaseFollowedHistory(sessionIdRef.current);
    }, 750);
  };

  // Bound a long-running open session without disturbing a detached reader.
  // Trimming is batched and old rows remain recoverable through `loadOlder`.
  useEffect(() => {
    const el = parentRef.current;
    // Do not fight the short-history bootstrap above: it deliberately pages
    // until the viewport fills. Once there is real overflow, the recent tail is
    // already sufficient to fill the reader and can safely replace deep rows.
    if (
      managesScrollHistory &&
      selectedToolKey === null &&
      stick.current &&
      el &&
      el.scrollHeight > el.clientHeight + 1
    ) {
      scheduleHistoryRelease();
    }
    return cancelHistoryRelease;
  }, [managesScrollHistory, sessionId, timeline.length, selectedToolKey]);

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
    let touching = false;
    let magneticArmed = false;
    let directManipulationActive = false;
    let directManipulationShouldFollow = false;
    let nativeScrollSettleTimer: number | undefined;
    const magneticThreshold = (): number => {
      const lineHeight = Number.parseFloat(globalThis.getComputedStyle(el).lineHeight) || 24;
      return Math.max(40, lineHeight * 2);
    };
    const detach = (): void => {
      cancelHistoryRelease();
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
    const captureAnchor = (): void =>
      captureFreezeAnchor(el, freezeRef.current);
    const saveViewport = (): void => {
      const mode = managesScrollHistoryRef.current ? "history" : "page";
      if (mode === "page" && !pageIdRef.current) return;
      saveTranscriptViewport({
        sessionId: sessionIdRef.current,
        mode,
        pageId: mode === "page" ? pageIdRef.current ?? null : null,
        anchorKey: stick.current ? null : freezeRef.current.key,
        anchorOffset: freezeRef.current.top,
        scrollOffset: el.scrollTop,
        following: mode === "history" && stick.current,
      });
    };
    const restoreAnchor = (): void =>
      restoreFreezeAnchor(el, freezeRef.current);
    const markNativeScrollActive = (): void => {
      cancelHistoryRelease();
      markTranscriptScrollActivity();
      nativeScrollActiveRef.current = true;
      setRenderPausedForScroll(true);
      // A pending corrective scroll event must never swallow the reader's next
      // real movement. From this point the native gesture is authoritative.
      freezeRef.current.self = false;
      if (nativeScrollSettleTimer !== undefined) {
        globalThis.clearTimeout(nativeScrollSettleTimer);
      }
      nativeScrollSettleTimer = globalThis.setTimeout(() => {
        const fromBottom = Math.abs(el.scrollTop);
        if (
          shouldMagnetizeTranscript({
            history: managesScrollHistoryRef.current,
            working: workingRef.current,
            detached: !stick.current,
            touching,
            fromBottom,
            threshold: magneticThreshold(),
          })
        ) {
          stick.current = true;
          setSticky(sessionIdRef.current, true);
          freezeRef.current.key = null;
          el.scrollTo({
            top: 0,
            behavior: globalThis.matchMedia("(prefers-reduced-motion: reduce)").matches
              ? "auto"
              : "smooth",
          });
          saveViewport();
          scheduleHistoryRelease();
          nativeScrollSettleTimer = globalThis.setTimeout(() => {
            nativeScrollActiveRef.current = false;
            setRenderPausedForScroll(false);
            nativeScrollSettleTimer = undefined;
          }, 360);
          return;
        }
        // Capture first, then hand ownership back to chunk/resize anchoring.
        // The next stream update therefore preserves the exact settled view.
        if (!stick.current) captureAnchor();
        saveViewport();
        nativeScrollActiveRef.current = false;
        setRenderPausedForScroll(false);
        nativeScrollSettleTimer = undefined;
      }, 240);
    };
    const onTouchStart = (): void => {
      viewportRestoreActiveRef.current = false;
      // A real transcript scroll supersedes an in-flight drawer catch-up. The
      // native-scroll settle path will atomically adopt the latest canonical
      // timeline after the reader's gesture ends.
      drawerCatchupActiveRef.current = false;
      touching = true;
      // Each new finger gesture is one explicit request opportunity. A page
      // landing while the reader remains near the beginning must not recursively
      // drain the transcript, but the next upward swipe should fetch the next
      // page without first forcing the reader away from the threshold.
      historyPrefetchArmedRef.current = true;
      markNativeScrollActive();
      detach();
      const fromBottom = Math.abs(el.scrollTop);
      const prefetch = historyPrefetchTransition({
        managed: managesScrollHistoryRef.current,
        detached: true,
        armed: historyPrefetchArmedRef.current,
        fromTop: el.scrollHeight - el.clientHeight - fromBottom,
        // Begin the bounded skeleton fill before the reader reaches the
        // retained boundary. Three viewports gives mobile radios and WebKit
        // enough lead time without draining history.
        threshold: el.clientHeight * 3,
      });
      historyPrefetchArmedRef.current = prefetch.armed;
      if (prefetch.request) requestOlderPageRef.current();
    };
    const onTouchEnd = (): void => {
      touching = false;
    };
    const onDirectManipulationStart = (): void => {
      if (directManipulationActive) return;
      directManipulationActive = true;
      drawerCatchupActiveRef.current = false;
      // Transcript's own touchstart runs before the app-shell bubble listener
      // can direction-lock the drawer gesture. It may already have detached a
      // bottom-following reader, so infer that original intent from geometry.
      directManipulationShouldFollow = stick.current ||
        (
          Math.abs(el.scrollTop) <= 1 &&
          (managesScrollHistoryRef.current || workingRef.current)
        );
      cancelHistoryRelease();
      markTranscriptScrollActivity();
      nativeScrollActiveRef.current = true;
      setRenderPausedForScroll(true);
      if (nativeScrollSettleTimer !== undefined) {
        globalThis.clearTimeout(nativeScrollSettleTimer);
        nativeScrollSettleTimer = undefined;
      }
    };
    const onDirectManipulationEnd = (): void => {
      if (!directManipulationActive) return;
      directManipulationActive = false;
      if (directManipulationShouldFollow) {
        stick.current = true;
        setSticky(sessionIdRef.current, true);
        freezeRef.current.key = null;
        saveViewport();
        scheduleHistoryRelease();
      } else {
        captureAnchor();
        saveViewport();
      }
      directManipulationShouldFollow = false;
      nativeScrollActiveRef.current = false;
      // Do not replace the entire accumulated timeline in one commit. Live text
      // chunks are coalesced into one growing envelope, so a single "latest"
      // flush can still require a large Markdown/DOM reconciliation. Advance the
      // frozen presentation through bounded frames instead.
      startDrawerCatchupRef.current();
    };
    const onWheel = (event: WheelEvent): void => {
      // A wheel/trackpad gesture that continues toward the bottom can still
      // emit events after `scroll` has re-enabled Following. Detaching for
      // every wheel event therefore produced an on -> off flicker at the
      // boundary. Only an upward (away-from-latest) gesture expresses intent
      // to pause following.
      markNativeScrollActive();
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
      // Page/session mounting writes scrollTop while lazy rows and Markdown
      // settle. Those synthetic scroll events are not reading intent and must
      // not overwrite the saved viewport before restoration completes.
      if (viewportRestoreActiveRef.current) {
        reportScrollableRef.current();
        return;
      }
      // Covers scrollbar drags and keyboard/native scrolling as well as wheel
      // gestures. Debouncing here keeps inertia authoritative between events.
      markNativeScrollActive();
      // column-reverse: the bottom is scrollTop 0 (abs handles the sign).
      const fromBottom = Math.abs(el.scrollTop);
      const insideMagneticZone = !stick.current &&
        (managesScrollHistoryRef.current || workingRef.current) &&
        fromBottom <= magneticThreshold();
      const magnetic = magneticHapticTransition(
        magneticArmed,
        fromBottom,
        magneticThreshold(),
      );
      if (insideMagneticZone && magnetic.fire) magneticHaptic();
      magneticArmed = magnetic.armed;
      // Detached → keep the freeze anchor fresh as the reader scrolls, so the
      // moment they stop, the held position is exactly where they left off.
      if (!stick.current) captureAnchor();
      saveViewport();
      // Fetch one page per deliberate entry into the top threshold. Remaining
      // inside the zone while a page lands must not chain through the entire
      // session; the reader must move away and approach the top again.
      const fromTop = el.scrollHeight - el.clientHeight - fromBottom;
      const prefetch = historyPrefetchTransition({
        managed: managesScrollHistoryRef.current,
        detached: !stick.current,
        armed: historyPrefetchArmedRef.current,
        fromTop,
        threshold: el.clientHeight * 3,
      });
      historyPrefetchArmedRef.current = prefetch.armed;
      if (prefetch.request) {
        requestOlderPageRef.current();
      }
      reportScrollableRef.current();
    };
    const awayDirection = (): 1 | -1 => {
      if (el.scrollTop > 0.5) return 1;
      if (el.scrollTop < -0.5) return -1;
      // column-reverse uses opposite scrollTop signs across engines. Probe one
      // pixel from the bottom once instead of UA-sniffing Chrome vs WebKit.
      el.scrollBy({ top: -1, behavior: "auto" });
      if (el.scrollTop < -0.5) return -1;
      el.scrollBy({ top: 1, behavior: "auto" });
      return el.scrollTop > 0.5 ? 1 : -1;
    };
    const scrollAway = (distance: number): void => {
      detach();
      const direction = awayDirection();
      el.scrollBy({ top: direction * distance, behavior: "auto" });
    };
    const scrollTowardLatest = (distance: number): void => {
      const fromBottom = Math.abs(el.scrollTop);
      if (fromBottom <= distance) {
        el.scrollTop = 0;
        stick.current = true;
        setSticky(sessionIdRef.current, true);
        freezeRef.current.key = null;
        scheduleHistoryRelease();
        return;
      }
      const direction = el.scrollTop > 0 ? -1 : 1;
      el.scrollBy({ top: direction * distance, behavior: "auto" });
    };
    const followLatest = (): void => {
      stick.current = true;
      freezeRef.current.key = null;
      scheduleHistoryRelease();
      requestStickToBottom(sessionIdRef.current);
    };
    const onExploreCurrentPageBottom = (rawEvent: Event): void => {
      const requestedSessionId = (
        rawEvent as CustomEvent<{ sessionId?: string }>
      ).detail?.sessionId;
      if (requestedSessionId !== sessionIdRef.current) return;
      if (workingRef.current) {
        // The newest page is actively streaming: preserve History View's
        // Following toggle semantics.
        if (stick.current) detach();
        else followLatest();
        return;
      }
      // A settled or older Explore page is a bounded reading surface. Move to
      // the end of that page without enabling Following or changing pages.
      freezeRef.current.key = null;
      el.scrollTo({ top: 0, behavior: "smooth" });
    };
    const onExplorePageStart = (rawEvent: Event): void => {
      const requestedSessionId = (
        rawEvent as CustomEvent<{ sessionId?: string }>
      ).detail?.sessionId;
      if (requestedSessionId !== sessionIdRef.current) return;
      // Page navigation is a reading action. Keep Transcript's private follow
      // intent in sync with the shared sticky store before Explore positions
      // the question root. Otherwise a live answer immediately pulls the
      // reader back to the newest token and the first follow-button tap merely
      // disables that stale intent.
      detach();
      freezeRef.current.key = null;
    };
    const onSaveViewport = (): void => {
      // Session switching can happen while iOS momentum is still active, before
      // the 240ms settle timer captures its final position. Snapshot in the
      // switching tap's task while refs still identify the outgoing session.
      if (!stick.current) captureAnchor();
      saveViewport();
    };
    const onDesktopNavigation = (rawEvent: Event): void => {
      const action = (rawEvent as CustomEvent<{ action?: string }>).detail
        ?.action;
      const line = Math.max(
        32,
        Number.parseFloat(globalThis.getComputedStyle(el).lineHeight) || 24,
      );
      const halfPage = Math.max(line, el.clientHeight * 0.5);
      const page = Math.max(line, el.clientHeight * 0.9);
      if (action === "line-up") scrollAway(line);
      else if (action === "line-down") scrollTowardLatest(line);
      else if (action === "half-page-up") scrollAway(halfPage);
      else if (action === "half-page-down") scrollTowardLatest(halfPage);
      else if (action === "page-up") scrollAway(page);
      else if (action === "page-down") scrollTowardLatest(page);
      else if (action === "oldest") {
        detach();
        el.scrollTo({
          top: awayDirection() * el.scrollHeight,
          behavior: "auto",
        });
      } else if (action === "latest") followLatest();
      else if (action === "toggle-following") {
        if (stick.current) detach();
        else followLatest();
      }
    };
    el.addEventListener("wheel", onWheel, { passive: true });
    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: true });
    el.addEventListener("touchcancel", onTouchEnd, { passive: true });
    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("cowboy:desktop-transcript-nav", onDesktopNavigation);
    globalThis.addEventListener(
      "cowboy:explore-current-page-bottom",
      onExploreCurrentPageBottom,
    );
    globalThis.addEventListener(
      "cowboy:explore-page-start",
      onExplorePageStart,
    );
    globalThis.addEventListener(
      "cowboy:transcript-save-viewport",
      onSaveViewport,
    );
    globalThis.addEventListener(
      "cowboy:transcript-direct-manipulation-start",
      onDirectManipulationStart,
    );
    globalThis.addEventListener(
      "cowboy:transcript-direct-manipulation-end",
      onDirectManipulationEnd,
    );
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
      if (roTries === 0 && Math.abs(el.scrollTop) > 0.5) {
        // Queue/draft disclosure changes the transcript viewport. Let WebKit's
        // scrolling thread carry the short correction instead of visibly
        // teleporting the final lines; later frames still converge exactly.
        el.scrollTo({ top: 0, behavior: "smooth" });
      } else {
        pinTranscriptToLatest(el); // column-reverse: 0 = bottom
      }
      if (++roTries < 5) roRaf = requestAnimationFrame(repin);
    };
    const ro = new ResizeObserver(() => {
      reportScrollableRef.current(); // viewport resized → overflow may have flipped
      const previousHeight = viewportHeightRef.current;
      const nextHeight = el.clientHeight;
      viewportHeightRef.current = nextHeight;
      if (
        !desktopNavigation &&
        previousHeight !== null &&
        nextHeight > previousHeight + 1
      ) {
        viewportBackfillAllowanceRef.current = VIEWPORT_BACKFILL_PAGE_LIMIT;
        setViewportBackfillPaused(false);
        requestViewportBackfillRef.current(true);
      }
      if (!stick.current) {
        // Detached: don't follow the bottom — hold the reader's view against the
        // streaming bottom bubble's upward growth (see FREEZE-WHILE-DETACHED).
        if (
          shouldRestoreDetachedAnchor(
            desktopNavigation,
            nativeScrollActiveRef.current,
          )
        ) restoreAnchor();
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
      el.removeEventListener(
        "cowboy:desktop-transcript-nav",
        onDesktopNavigation,
      );
      globalThis.removeEventListener(
        "cowboy:explore-current-page-bottom",
        onExploreCurrentPageBottom,
      );
      globalThis.removeEventListener(
        "cowboy:explore-page-start",
        onExplorePageStart,
      );
      globalThis.removeEventListener(
        "cowboy:transcript-save-viewport",
        onSaveViewport,
      );
      globalThis.removeEventListener(
        "cowboy:transcript-direct-manipulation-start",
        onDirectManipulationStart,
      );
      globalThis.removeEventListener(
        "cowboy:transcript-direct-manipulation-end",
        onDirectManipulationEnd,
      );
      el.removeEventListener("keydown", onKeyDown);
      ro.disconnect();
      if (nativeScrollSettleTimer !== undefined) {
        globalThis.clearTimeout(nativeScrollSettleTimer);
      }
      cancelHistoryRelease();
      drawerCatchupActiveRef.current = false;
      nativeScrollActiveRef.current = false;
      setRenderPausedForScroll(false);
      if (roRaf !== 0) cancelAnimationFrame(roRaf);
      if (viewportBackfillRafRef.current !== 0) {
        cancelAnimationFrame(viewportBackfillRafRef.current);
        viewportBackfillRafRef.current = 0;
      }
    };
  }, []);

  // Restore a device-local reading anchor when returning to a session. History
  // keeps its own anchor across projection changes. Page navigation and entry
  // into Page View clear the page cache, so only a session round-trip can
  // restore a Page position; a different/newly opened page starts at its head.
  useLayoutEffect(() => {
    viewportRestoreActiveRef.current = true;
    if (restoringProjectionMountRef.current) {
      stick.current = false;
      setSticky(sessionId, false);
      viewportRestoreActiveRef.current = false;
      return undefined;
    }
    const mode = managesScrollHistory ? "history" : "page";
    const saved = getTranscriptViewport(sessionId, mode);
    const canRestore = canRestoreTranscriptViewport(
      saved,
      mode,
      pageId ?? null,
    );
    const restoreDetached = canRestore && !saved.following &&
      saved.anchorKey !== null;
    const restoreOffset = canRestore && !saved.following;
    stick.current = canRestore ? saved.following : mode === "history";
    if (stick.current) resetSticky(sessionId);
    else setSticky(sessionId, false);
    let raf = 0;
    let tries = 0;
    let stableFrames = 0;
    let previousHeight = -1;
    const position = (): void => {
      if (!viewportRestoreActiveRef.current) return;
      const el = parentRef.current;
      if (el && restoreOffset) {
        // Exact offset is the reliable fallback while the cached anchor row is
        // not mounted yet (page projection/history hydration can take frames).
        el.scrollTop = saved.scrollOffset;
      }
      if (el && restoreDetached) {
        const anchor: FreezeAnchor = {
          key: saved.anchorKey,
          top: saved.anchorOffset,
          self: false,
        };
        restoreFreezeAnchor(el, anchor);
        freezeRef.current = anchor;
      } else if (el && stick.current) {
        pinTranscriptToLatest(el);
      } else if (el && mode === "page" && !restoreOffset) {
        el.scrollTop = el.clientHeight - el.scrollHeight;
      }
      if (el) {
        const expected = restoreOffset
          ? saved.scrollOffset
          : stick.current
          ? 0
          : el.clientHeight - el.scrollHeight;
        const stable = Math.abs(el.scrollHeight - previousHeight) < 0.5 &&
          Math.abs(el.scrollTop - expected) < 0.5;
        stableFrames = stable ? stableFrames + 1 : 0;
        previousHeight = el.scrollHeight;
      }
      tries += 1;
      // Lazy Markdown, fonts and images can commit after the first dozen
      // frames. Hold the requested viewport through a real stability window,
      // but never longer than ~1.5s; touchstart cancels immediately above.
      if ((tries < 30 || stableFrames < 8) && tries < 90) {
        raf = requestAnimationFrame(position);
      } else {
        viewportRestoreActiveRef.current = false;
      }
    };
    raf = requestAnimationFrame(position);
    return () => {
      cancelAnimationFrame(raf);
      viewportRestoreActiveRef.current = false;
    };
  }, [managesScrollHistory, pageId, sessionId]);

  // A live Page is allowed to follow only for the duration of its active turn.
  // As soon as that turn settles, freeze it as an ordinary reading page at its
  // current position. This also clears the shared highlight state so returning
  // to History cannot inherit a stale Page-only follow intent.
  useLayoutEffect(() => {
    const key = `${sessionId}:${pageId ?? ""}`;
    const previous = pageWorkingRef.current;
    pageWorkingRef.current = { key, working };
    if (
      managesScrollHistory || previous.key !== key ||
      !previous.working || working
    ) return undefined;

    stick.current = false;
    setSticky(sessionId, false);
    let raf = 0;
    let tries = 0;
    const freezeSettledPage = (): void => {
      const el = parentRef.current;
      if (el) {
        captureFreezeAnchor(el, freezeRef.current);
        saveTranscriptViewport({
          sessionId,
          mode: "page",
          pageId: pageId ?? null,
          anchorKey: freezeRef.current.key,
          anchorOffset: freezeRef.current.top,
          scrollOffset: el.scrollTop,
          following: false,
        });
      }
      if (++tries < 5) raf = requestAnimationFrame(freezeSettledPage);
    };
    raf = requestAnimationFrame(freezeSettledPage);
    return () => cancelAnimationFrame(raf);
  }, [managesScrollHistory, pageId, sessionId, working]);

  // After a PRESENTED timeline change. Normally this fires per streamed chunk;
  // while native scrolling is active presentation is frozen, then the latest
  // canonical timeline is flushed atomically when scrolling settles. Keying
  // this effect to `presentedTimeline` (not the continuously advancing backing
  // `timeline`) is essential: the flush must restore the detached anchor before
  // paint, otherwise the accumulated streamed content visibly shifts the reader
  // once at every scroll-settle boundary.
  // STUCK: follow the bottom, scrollTop 0 (a no-op when the native bottom anchor
  // already held it there, a safety net if it didn't).
  // DETACHED: hold the reader's view against the streaming bottom bubble's upward
  // growth by re-asserting the freeze anchor (column-reverse's native anchor only
  // pins the bottom, which is exactly what a scrolled-up reader does NOT want).
  // Pre-paint (layout effect) so the correction lands without a visible jump.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    if (stick.current) {
      pinTranscriptToLatest(el);
      // Stuck → no live anchor; clear so the next detach captures fresh (never
      // restores to a stale, pre-re-stick position → a jump).
      freezeRef.current.key = null;
    } else if (
      shouldRestoreDetachedAnchor(
        desktopNavigation,
        nativeScrollActiveRef.current,
      ) && freezeRef.current.key === null
    ) {
      // Detached without a scroll gesture to capture one (e.g. the composer's
      // sticky toggle) → seed the anchor at the current view this first chunk.
      captureFreezeAnchor(el, freezeRef.current);
    } else if (
      shouldRestoreDetachedAnchor(
        desktopNavigation,
        nativeScrollActiveRef.current,
      )
    ) {
      restoreFreezeAnchor(el, freezeRef.current);
    }
    // Content grew/shrank → overflow may have flipped, and neither the RO (the
    // viewport didn't resize) nor `scroll` fires. Re-measure AFTER paint: the
    // newly visible contained rows lay out over the next frame, so
    // `scrollHeight` is briefly stale this layout pass.
    const sRaf = requestAnimationFrame(() => reportScrollableRef.current());
    return () => cancelAnimationFrame(sRaf);
  }, [presentedTimeline]);

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
      if (el) pinTranscriptToLatest(el);
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
        data-transcript-session={sessionId}
        data-render-paused={renderPausedForScroll ? "true" : undefined}
        data-desktop-transcript-scroller={desktopNavigation
          ? "true"
          : undefined}
        tabIndex={0}
        sx={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          // The mobile session drawer owns a non-passive horizontal touch
          // recognizer on an ancestor. Without an explicit axis contract,
          // iOS WebKit keeps some long Page View gestures on the main thread
          // while it waits for that recognizer, and native vertical scrolling
          // can fail to start even though this element has real overflow.
          // Commit vertical movement to the async scroller immediately; a
          // horizontal drag remains available to the drawer recognizer.
          touchAction: "pan-y pinch-zoom",
          WebkitOverflowScrolling: "touch",
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
          // `bottomInset` is the one settled border-box measurement for the
          // complete floating stack (status, Plan, Pending, Composer and the
          // bottom navbar when present). Transcript must not recombine child
          // heights or add another boundary token here.
          pb: bottomInset ?? "12px",
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
          ...(desktopNavigation ? desktopScrollbarSx : {}),
          // This is a scrolling FLEX column, not a height-constrained toolbar.
          // Flex items default to `flex-shrink: 1`; while a streamed message is
          // growing and the next tool card lands, WebKit can briefly keep the
          // message's old flex base size and paint the new lines outside that
          // shrunken row. The result is the prose visibly crossing the next
          // card until another layout pass. Transcript rows must always own
          // their intrinsic content height so overflow increases scrollHeight
          // instead of compressing siblings.
          "& > *": { flex: "0 0 auto" },
          // The loading outline is the sole intentional flexible child. A zero
          // basis + minHeight:0 means it can consume exactly the unused area but
          // contributes no intrinsic height once real transcript rows fill it.
          "& > [data-transcript-loading-fill]": {
            flex: "1 1 0",
            minHeight: 0,
          },
        }}
      >
        {status === "starting" && items.length === 0
          ? <PreparingTranscript provider={provider} cwd={cwd} />
          : loading && items.length === 0
          ? (
            <TranscriptSkeleton
              desktop={desktopNavigation}
              provider={provider}
            />
          )
          : showFreshSessionEmptyState
          ? <EmptyTranscript provider={provider} cwd={cwd} />
          : (
            // Rendered NEWEST-FIRST in the DOM; column-reverse flips it to
            // oldest-at-top / newest-at-bottom on screen. The trailing dots are
            // DOM-first → the very bottom (below the newest item). Keyed by the
            // item's STABLE key (first envelope seq) so prepending older history
            // doesn't re-mount/jump rows.
            <>
              {/* column-reverse makes the first DOM child visually lowest.
                  Keep the page-turn footer next to the persistent Page Dock;
                  the flexible remainder belongs between short content and its
                  footer, never below the footer as a large dead zone. */}
              {pageFooter}
              {shortContentAtTop && (
                <Box
                  aria-hidden
                  data-transcript-page-spacer
                  sx={{
                    flex: "1 1 0 !important",
                    minHeight: 0,
                    pointerEvents: "none",
                    userSelect: "none",
                    WebkitUserSelect: "none",
                  }}
                />
              )}
              {showJudging && <TranscriptJudgingActivity />}
              {
                /* Still-waiting row: after QUIET_BADGE_MIN of no timeline activity on a
                working turn, surface the silence (count-up) + a REAL red Stop button.
                cowboy no longer auto-kills a silent turn (see acp.rs) — the human
                decides, so the recovery action is a first-class control here. */
              }
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
                      importantHaptic();
                      send({ type: "cancel", session_id: sessionId });
                    }}
                    sx={{ textTransform: "none", minHeight: 28, py: 0.25 }}
                  >
                    中断
                  </Button>
                </Box>
              )}
              {showTrailingDots && (
                <Box
                  sx={{ py: 0.625, display: "flex", flexDirection: "column" }}
                >
                  <ThinkingIndicator provider={provider} />
                </Box>
              )}
              {compacting && (
                <Box
                  sx={{ py: 0.625, display: "flex", flexDirection: "column" }}
                >
                  <CompactingWidget
                    active
                    provider={provider}
                    desktop={desktopNavigation}
                  />
                </Box>
              )}
              {
                /* Optimistic chat bubbles: newest-first in the DOM (column-reverse →
                they sit just above the latest real item / below the dots). */
              }
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
                      // Give every timeline row an independent paint boundary.
                      // WebKit can otherwise retain a previous composited tool
                      // card for one frame while a streamed sibling thought is
                      // inserted/reflowed, visibly drawing the two rows on top of
                      // each other even though their layout boxes do not overlap.
                      // This does not size-contain the row: intrinsic Markdown and
                      // tool height still contribute normally to scrollHeight.
                      contain: "layout paint",
                      color: locatedToolKey === item.key ? "primary.main" : undefined,
                      animation: locatedToolKey === item.key
                        ? `${toolLocateFlash} 1.4s ease-out`
                        : undefined,
                      borderRadius: locatedToolKey === item.key ? 1 : undefined,
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
                      desktop={desktopNavigation}
                      selectedToolKey={selectedToolKey}
                      onOpenTool={openTool}
                    />
                  </Box>
                ))}
              {managesScrollHistory && !backfillingViewport && paging != null &&
                  paging.beforeSeq !== null && !paging.reachedStart && (
                <ScrollbackLoadingSkeleton
                  height={scrollbackFailed
                    ? 44
                    : scrollbackLoading
                    ? scrollbackFillHeight
                    : SCROLLBACK_IDLE_BOUNDARY_HEIGHT}
                  loading={scrollbackLoading}
                  failed={scrollbackFailed}
                  onRetry={() => {
                    scrollbackRetryCountRef.current = 0;
                    setScrollbackFailed(false);
                    requestOlderPageRef.current();
                  }}
                />
              )}
              {!desktopNavigation &&
                showHistoryLoadingFill && (
                <TranscriptLoadingFill
                  label={viewportBackfillPaused
                    ? "Earlier conversation is available"
                    : "Loading conversation data"}
                  paused={viewportBackfillPaused}
                  onContinue={viewportBackfillPaused
                    ? () => {
                      viewportBackfillAllowanceRef.current =
                        VIEWPORT_BACKFILL_PAGE_LIMIT;
                      viewportBackfillSettlingRef.current = false;
                      setViewportBackfillPaused(false);
                      requestViewportBackfillRef.current(false);
                    }
                    : undefined}
                />
              )}
            </>
          )}
      </Box>
      <ToolDetailsBrowser
        items={items}
        runs={runs}
        selectedKey={selectedToolKey}
        desktop={desktopNavigation}
        provider={provider}
        onSelect={setSelectedToolKey}
        onClose={closeTool}
        onLocate={locateTool}
        historyComplete={paging?.reachedStart ?? true}
      />
      {
        /* Persistent bottom strip: interrupted / crashed / dormant / disconnected.
          In-flow (flexShrink:0) so it sits below the scroll area, above the
          composer — never covering the last message. */
      }
      <SessionStatusBar status={status} crashDetail={crashDetail} />
      {
        /* The scroll-to-latest affordance is the persistent sticky/auto-scroll
          toggle in the composer (stickyStore + Composer), not a pill here. A
          pending permission is surfaced by the PermissionOverlay in the composer. */
      }
    </Box>
  );
}
