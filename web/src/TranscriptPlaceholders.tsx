import { desktopSize, glyphSize } from "./surface/desktopSize";
import { SessionStartSettings } from "./SessionStartSettings";
import { useEffect, useState } from "react";
import { Box, Button, ButtonBase, CircularProgress, Skeleton, Stack, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";
import { shortcutKey } from "./desktop/commands/imeShortcut";
import { ChatBubbleOutline, CleaningServices, Terminal, Tune } from "@mui/icons-material";
import { SessionContinuedFrom } from "./SessionContextCue";
import { providerPresentation } from "./providerPresentation";
import { useStoreSelector } from "./store";
import { prepareSweep } from "./sessionPreparing";
import { CONVERSATION_SKELETON_TURNS, transcriptRestoreCaption } from "./transcriptLoadingPresentation";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { openSessionSettings } from "./sessionSettingsOpen";
import { PreparingConversation } from "./PreparingConversation";

// --- Loading primitives -----------------------------------------------------

// Chat-history skeleton shown while a session's snapshot is still in flight (the
// startup blank). Placeholder turns — assistant blocks as left-aligned prose
// lines, "mine" blocks as a right-aligned bubble — so it reads as a
// conversation, not a form. Sizes are %/maxWidth-relative and it rides inside
// the transcript's padded reading column, so the same markup gives the right
// gutter + line width on iPhone, iPad and desktop with no breakpoints.

export function TranscriptSkeleton({
  desktop,
  provider,
  providerVersion,
  providerDigest,
  onRetry,
}: {
  desktop: boolean;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  onRetry?: (() => void) | undefined;
}): React.JSX.Element {
  const [stalled, setStalled] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const timer = globalThis.setTimeout(() => setStalled(true), 8_000);
    return () => globalThis.clearTimeout(timer);
  }, [attempt]);

  const agent =
    providerPresentation(provider, providerVersion, providerDigest).agent;

  return (
    <Stack
      data-transcript-switch-skeleton
      spacing={desktop ? 3 : 2.25}
      sx={{
        minHeight: desktop ? undefined : "100%",
        py: desktop ? 2 : { xs: 2, sm: 3 },
        justifyContent: desktop
          ? undefined
          : { xs: "flex-end", sm: "flex-start" },
      }}
      aria-busy="true"
      aria-label="Loading chat history"
    >
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        sx={{ color: "text.secondary" }}
      >
        <CircularProgress
          size={desktopSize(14)}
          thickness={4}
          color="inherit"
          aria-hidden
        />
        <Typography variant="caption" sx={{ fontWeight: 650 }}>
          {transcriptRestoreCaption("hydrate", agent)}
        </Typography>
      </Stack>
      {CONVERSATION_SKELETON_TURNS.map((turn, i) => (
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
                animation="wave"
                width={turn.lines[0]}
                height={38}
                sx={{ maxWidth: "75%", borderRadius: 2.5 }}
              />
            )
            : (
              turn.lines.map((w, j) => (
                <Skeleton
                  key={j}
                  variant="rounded"
                  animation="wave"
                  width={w}
                  height={12}
                  sx={{ borderRadius: 99 }}
                />
              ))
            )}
        </Stack>
      ))}
      {stalled && onRetry !== undefined && (
        <Button
          size="small"
          variant="text"
          onClick={(): void => {
            setStalled(false);
            setAttempt((current) => current + 1);
            onRetry();
          }}
          sx={{ alignSelf: "flex-start", minHeight: 36, textTransform: "none" }}
        >
          Taking a while — retry
        </Button>
      )}
    </Stack>
  );
}

// During the first older-page restore, turn otherwise ambiguous unused space
// into a quiet loading outline. Never show it merely because the agent is
// working: the real status/thinking/tool rows already describe execution, and
// a restore caption over a live turn looks like history is still arriving.
// This is a FLEX filler, not
// guessed history height: it consumes only free viewport space, shrinks
// one-for-one as real rows grow, and reaches zero before the transcript
// overflows. It therefore never adds scroll range or disturbs iOS' anchor.
export function TranscriptLoadingFill({
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
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        sx={{
          flexShrink: 0,
          px: { xs: 2.5, sm: 4 },
          pt: { xs: 1.25, sm: 1.75 },
          pb: 0.75,
          maxWidth: 760,
          mx: "auto",
          width: "100%",
          color: "text.secondary",
        }}
      >
        {paused ? null : (
          <CircularProgress
            size={desktopSize(13)}
            thickness={4}
            color="inherit"
            aria-hidden
          />
        )}
        <Typography variant="caption" sx={{ fontWeight: 650 }}>
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
      <Stack
        sx={{
          flex: 1,
          minHeight: 0,
          px: { xs: 2.5, sm: 4 },
          pb: { xs: 1.5, sm: 2.5 },
          mx: "auto",
          width: "100%",
          maxWidth: 760,
          justifyContent: "space-evenly",
          gap: { xs: 1.15, sm: 1.6 },
          opacity: paused ? 0.48 : 0.78,
          maskImage:
            "linear-gradient(to bottom, black 0%, black 82%, transparent 100%)",
          WebkitMaskImage:
            "linear-gradient(to bottom, black 0%, black 82%, transparent 100%)",
        }}
      >
        {CONVERSATION_SKELETON_TURNS.map(({ mine, lines }, turn) => (
          <Stack
            // Static outline with no reordering.
            key={turn}
            spacing={0.7}
            sx={{
              width: mine ? "58%" : "100%",
              alignSelf: mine ? "flex-end" : "stretch",
              p: mine ? 1.1 : 0,
              borderRadius: mine ? 2.5 : 0,
              "@media (min-width: 600px)": {
                width: mine ? "44%" : "100%",
                maxWidth: mine ? 360 : "none",
                p: mine ? 1.2 : 0,
              },
              bgcolor: mine
                ? (theme) => alpha(theme.palette.primary.main, 0.05)
                : "transparent",
            }}
          >
            {lines.map((width) => (
              <Skeleton
                key={width}
                variant="rounded"
                animation={paused ? false : "wave"}
                width={width}
                height={mine ? 14 : 11}
                sx={{
                  borderRadius: 99,
                  bgcolor: (theme) =>
                    alpha(theme.palette.text.secondary, mine ? 0.08 : 0.12),
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
export function ScrollbackLoadingSkeleton({
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
  const rows = ["58%", "42%", "51%", "63%", "37%"];
  return (
    <Box
      data-transcript-scrollback-fill
      role="status"
      aria-live="polite"
      aria-label={transcriptRestoreCaption("backfill")}
      sx={{
        height: `${Math.max(0, height)}px`,
        minHeight: 0,
        overflow: "hidden",
        pointerEvents: failed ? "auto" : "none",
        userSelect: "none",
        WebkitUserSelect: "none",
        transition: "height 160ms ease-out, opacity 140ms ease",
        opacity: failed ? 1 : loading ? 0.72 : 0.42,
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
                  variant="rounded"
                  animation={loading ? "wave" : false}
                  width={title}
                  height={11}
                  sx={{ ml: 1.5, borderRadius: 99 }}
                />
                {loading && (
                  <Skeleton
                    variant="rounded"
                    animation="wave"
                    width={index % 2 === 0 ? "100%" : "62%"}
                    height={index % 3 === 1 ? 36 : 14}
                    sx={{
                      borderRadius: index % 3 === 1 ? 2.5 : 99,
                      ml: index % 2 === 0 ? 0 : "auto",
                    }}
                  />
                )}
              </Stack>
            ))}
          </Stack>
        )}
    </Box>
  );
}

// Empty conversations are usable workspaces, including before the first worker
// starts. Keep configuration reachable without requiring a throwaway message.

export type ConversationEmptyKind = "preparing" | "blocked" | "ready" | "cleared";

export interface ConversationEmptyContext {
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  project: string | null;
  machine: string | null;
  model: string | null;
  reasoning: string | null;
}

/** The empty-state context of a transcript whose host supplied none. */
export function fallbackConversationContext(input: {
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  cwd: string | null | undefined;
}): ConversationEmptyContext {
  const { cwd, ...provider } = input;
  return {
    ...provider,
    project: cwd ? cwd.replace(/\/+$/, "").split("/").at(-1) ?? null : null,
    machine: null,
    model: null,
    reasoning: null,
  };
}

export function focusPrimaryComposer(): void {
  document.querySelector<HTMLElement>(
    "[data-mobile-primary-composer] [data-mobile-editor-area] textarea, " +
      "[data-mobile-primary-composer] [data-mobile-editor-area] [contenteditable='true'], " +
      "[data-desktop-region='prompt.composer'] [contenteditable='true']",
  )?.focus();
}

export function ConversationEmptyState({
  sessionId,
  kind,
  preparing,
  sleeping = false,
  crashDetail = null,
  context,
  desktop,
}: {
  sessionId: string;
  kind: ConversationEmptyKind;
  // The agent is still booting (session status "starting"). For `kind ===
  // "preparing"` that IS the state; for cleared / fresh it is a modifier that
  // withdraws the readiness claim and adds the sweep bar. See
  // conversationEmptyPresentation.
  preparing: boolean;
  sleeping?: boolean;
  /** Why a `blocked` startup stopped. */
  crashDetail?: string | null;
  context: ConversationEmptyContext;
  desktop: boolean;
}): React.JSX.Element {
  const agent = providerPresentation(context.provider, context.providerVersion, context.providerDigest).agent;
  const location = [context.project, context.machine].filter(Boolean).join(" on ");
  const title = kind === "preparing"
    ? "Preparing session"
    : kind === "cleared"
    ? "New conversation"
    : "Start a conversation";
  // A cleared session restarts its agent: saying "is ready with a fresh context"
  // while the process is still spinning up is the contradiction this modifier
  // removes. Typing is still invited — a prompt written now is queued and the
  // daemon drains it on the Running edge.
  const readiness = preparing ? "is starting" : "is ready";
  // Once a first message is on its way there is nothing left to set up before
  // it: the page only has to say the wait is working, not offer a form.
  const committed = useStoreSelector((state) =>
    kind === "preparing" && (
      (state.queues.get(sessionId)?.length ?? 0) > 0 ||
      state.sessions.some((entry) =>
        entry.id === sessionId && entry.continued_from !== undefined
      )
    )
  );
  const detail = kind === "preparing"
    ? `${location
      ? `Creating an isolated workspace for ${location}.`
      : "Creating an isolated workspace."}${
      committed ? " Your message goes out as soon as it's ready." : ""
    }`
    : sleeping
    ? kind === "cleared"
      ? "Context cleared. Settings are saved; the agent starts when you send."
      : "The agent starts when you send."
    : kind === "cleared"
    ? `Previous context was cleared. ${agent} ${readiness} with a fresh context.`
    : location
    ? `${agent} ${readiness} in ${location}.`
    : preparing
    ? `${agent} is starting — you can write your first message now.`
    : `${agent} is ready for your first message.`;
  const facts = [
    agent,
    context.project,
    context.machine,
  ]
    .filter((value, index, values): value is string =>
      Boolean(value) && values.indexOf(value) === index
    );
  // Every empty state is a writing surface now: a prompt typed while the agent
  // boots is QUEUED (the daemon drains it on the Running edge), so tapping the
  // empty transcript to focus the composer is valid during startup too. Only the
  // live-region role still distinguishes them — the startup state is an
  // announcement, the others are landmarks.
  const announces = kind === "preparing";
  const settingsTap = useReliableTouchTap<HTMLButtonElement>(() => {
    openSessionSettings("agent", sessionId);
  });
  const writeFromEmpty = (event: { target: EventTarget | null }): boolean => {
    if (!(event.target instanceof Element)) return true;
    return event.target.closest("[data-conversation-empty-settings], [data-session-start-settings], [data-session-handoff]") ===
      null;
  };
  if (kind === "blocked") return <PreparingConversation sessionId={sessionId} location={location} blocked={{ detail: crashDetail }} />;
  if (kind === "preparing" && committed) return <PreparingConversation sessionId={sessionId} location={location} />;
  return (
    <Stack
      data-conversation-empty-state={kind}
      role={announces ? "status" : "region"}
      tabIndex={0}
      aria-live="polite"
      aria-label={preparing ? `${title}. ${detail}` : title}
      onClick={(event): void => {
        if (!writeFromEmpty(event)) return;
        focusPrimaryComposer();
      }}
      onKeyDown={(event): void => {
        const key = shortcutKey(event.nativeEvent);
        if (key !== "Enter" && key !== " ") return;
        if (!writeFromEmpty(event)) return;
        event.preventDefault();
        focusPrimaryComposer();
      }}
      sx={{
        // The reversed transcript anchors an intrinsic touch card above the
        // composer. Filling and centering it leaves a large gap when the
        // keyboard closes, and creates a second scroll owner on short screens.
        flex: desktop ? "1 1 0 !important" : "0 0 auto !important",
        minHeight: desktop ? 220 : 0,
        width: "100%",
        px: { xs: 2.5, sm: 3 },
        py: { xs: 2, sm: 3 },
        alignItems: "center",
        justifyContent: "safe center",
        overflowY: desktop ? "auto" : "visible",
        overscrollBehavior: "contain",
        "& > *": { flexShrink: 0 },
        textAlign: "center",
        gap: 1.1,
        color: "text.secondary",
        cursor: "text",
        userSelect: "none",
        WebkitUserSelect: "none",
        outline: "none",
        borderRadius: 3,
        transition: "background-color 180ms ease",
        "&:focus-visible": {
          bgcolor: (theme) => alpha(theme.palette.primary.main, 0.045),
        },
      }}
    >
      <SessionContinuedFrom sessionId={sessionId} />
      <Stack direction="row" alignItems="center" justifyContent="center" spacing={1} sx={{ width: "100%", maxWidth: 360 }}>
        <Box
          sx={{
            width: 32,
            height: 32,
            display: "grid",
            placeItems: "center",
            color: "primary.main",
          }}
        >
          {kind === "preparing"
            ? <Terminal sx={{ fontSize: glyphSize(22) }} />
            : kind === "cleared"
            ? <CleaningServices sx={{ fontSize: glyphSize(22) }} />
            : <ChatBubbleOutline sx={{ fontSize: glyphSize(22) }} />}
        </Box>
        <Typography
          variant="body1"
          sx={{ fontWeight: 600, fontSize: "1.125rem", color: "text.primary" }}
        >
          {title}
        </Typography>
      </Stack>
      {preparing && (
        <Typography variant="body2" sx={{ width: "100%", maxWidth: 360, lineHeight: 1.55 }}>
          {detail}
        </Typography>
      )}
      {preparing && (
        <Box
          aria-hidden
          sx={{
            position: "relative",
            width: "min(360px, 100%)",
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
              // The status palette, not the brand one: `starting` is info blue
              // everywhere else (App statusColor / StatusDot), so this bar and
              // the navbar dot read as one signal.
              bgcolor: "info.main",
              animation:
                `${prepareSweep} 1.65s cubic-bezier(.4,0,.2,1) infinite`,
            },
            "@media (prefers-reduced-motion: reduce)": {
              "&::after": { animation: "none", width: "55%", opacity: 0.72 },
            },
          }}
        />
      )}
      {!committed && (
        <ButtonBase
          data-conversation-empty-settings
          title="Session settings"
          aria-label={facts.length ? `Change session settings: ${facts.join(", ")}` : "Session settings"}
          aria-keyshortcuts="Enter Space"
          {...settingsTap}
          onPointerDown={(event): void => {
            event.stopPropagation();
            settingsTap.onPointerDown(event);
          }}
          onPointerUp={(event): void => {
            event.stopPropagation();
            settingsTap.onPointerUp(event);
          }}
          onClick={(event): void => {
            event.stopPropagation();
            settingsTap.onClick(event);
          }}
          sx={{
            width: "100%",
            maxWidth: 360,
            minHeight: desktopSize(44),
            justifyContent: "center",
            px: 0,
            py: 0.75,
            borderRadius: 3,
            cursor: "pointer",
            "&:active": { bgcolor: "action.selected" },
            "&:focus-visible": {
              outline: "2px solid",
              outlineColor: "primary.main",
              outlineOffset: 2,
            },
          }}
        >
          <Stack
            direction="row"
            useFlexGap
            flexWrap="wrap"
            justifyContent="center"
            alignItems="center"
            gap={0.75}
          >
            <Typography variant="body2" sx={{ color: "text.secondary", overflowWrap: "anywhere" }}>
              {facts.join(" · ") || "Session settings"}
            </Typography>
            <Tune
              aria-hidden
              sx={{ fontSize: glyphSize(16), color: "text.secondary", opacity: 0.78 }}
            />
          </Stack>
        </ButtonBase>
      )}
      {!committed && <SessionStartSettings key={sessionId} sessionId={sessionId} />}
    </Stack>
  );
}
