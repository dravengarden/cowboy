import { NoticeRow } from "./NoticeRow";
import { ScrollEdges } from "@cowboy/app-shell";
import { desktopSize, glyphSize } from "./surface/desktopSize";
import { ProtectedImage } from "./ProtectedImage";
import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Box, Button, CircularProgress, IconButton, keyframes, Paper, Stack, Tooltip, Typography, useTheme } from "@mui/material";
import { alpha, type Theme } from "@mui/material/styles";
import { DESKTOP_INSET_RADIUS } from "./desktop/DesktopEmbeddedControl";
import { shortcutKey } from "./desktop/commands/imeShortcut";
import { CheckCircleOutline, ChevronRight, CleaningServices, Close, CloudUpload, ErrorOutline, ExpandLess, ExpandMore, HelpOutlineRounded, Refresh, Undo, UnfoldLess, WarningAmberRounded } from "@mui/icons-material";
import { PendingCallRow, TaskRow } from "./TaskRow";
import { callKey, subagentKey } from "./backgroundTasks";
import { type BackgroundTasks } from "./useBackgroundTasks";
import { providerVisual } from "./providerVisual";
import { ProviderThoughtSteps } from "./ProviderTranscript";
import { Markdown } from "./Markdown";
import { attachmentDisplayParts, isLoadablePreviewUrl } from "./attachments";
import { applySendImagePreviews } from "./sendImagePreviews";
import { toolCopyText, mcpHeading, mcpIdentity } from "./tools/presentation";
import { formatShellForDisplay } from "./shellFormatter";
import { COMPACTING_NOTICE, type ContentChunk, isCompactionCommandText, isCompactionCompletionText, isHumanPrompt, type PromptOrigin, type RenderItem } from "./derive";
import { prettifyCrashDetail, rpcErrorKind } from "./crashDetail";
import { reportClientLog } from "./observability";
import { canContinueFailedTurn, TURN_CONTINUATION_PROMPT, turnFailureTelemetry } from "./turnRecovery";
import { PromptOriginNote } from "./PromptOriginNote";
import { TranscriptFileChip } from "./TranscriptFileChip";
import { ToolStatusChip } from "./tools/ToolStatusChip";
import type { Status } from "./protocol";
import { discardMessage, type QueuedMessage, retryMessage, returnFailedMessage, submitPrompt, useConnected } from "./store";
import { homeForOrigin, pendingSyncAppearance, returnLabelForHome } from "./localFirstDelivery";
import { USER_BUBBLE_COLLAPSE_BUFFER_PX, USER_BUBBLE_COLLAPSE_PX, userBubbleShouldClamp } from "./userBubbleCollapse";
import { MESSAGE_PREVIEW_MAX_WIDTH_PX, messageBubbleLayoutSx, messageBubbleSurfaceSx } from "./messageBubble";
import { transcriptLifecycleLabel } from "./transcriptLoadingPresentation";
import { ProtectedImageLightbox as ImageLightbox } from "./ProtectedImageLightbox";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { toolIcon } from "./ToolDetailsBrowser";
export const pulse = keyframes`
  0%, 100% { opacity: 1; }
  50%      { opacity: 0.55; }
`;

// Claude Code's "prompt keyword shimmer": a highlight band sweeps across the
// verb word (color applied via background-clip:text in sx).
export const shimmer = keyframes`to { background-position: -200% 0; }`;

// Blinking text caret — Claude.ai / Cursor put one at the end of streaming
// text so the user knows the model is still producing. `steps(2)` makes it
// blink-on / blink-off rather than fading, which reads as more "alive".
export const blink = keyframes`
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
`;

export function StreamingCaret(): React.JSX.Element {
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
export function isCompactingMessage(chunks: ContentChunk[]): boolean {
  if (chunks.length === 0) return false;
  let text = "";
  for (const c of chunks) {
    if (c.type !== "text") return false;
    text += c.text;
  }
  return text.trim() === COMPACTING_NOTICE;
}

export function isCompactionCommand(chunks: ContentChunk[]): boolean {
  if (chunks.length === 0 || chunks.some((chunk) => chunk.type !== "text")) {
    return false;
  }
  const command = chunks.map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("").trim();
  return isCompactionCommandText(command);
}

export function isCompactionCompletion(chunks: ContentChunk[]): boolean {
  if (chunks.length === 0 || chunks.some((chunk) => chunk.type !== "text")) {
    return false;
  }
  const text = chunks.map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("");
  return isCompactionCompletionText(text);
}

// Two states, because the notice is PERSISTED in the transcript and stays in
// scrollback after compaction ends:
//   • active (`active` — it's the live tail and the turn is still busy): a
//     terracotta shimmer + the shared activity mark = "condensing right now".
//   • done   (anything followed it / the turn is idle): a calm, static muted
//     "Context compacted" note — no infinite shimmer implying it's still going.
export function CompactingWidget({
  active,
  provider,
  providerVersion,
  providerDigest,
  desktop,
}: {
  active: boolean;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  desktop: boolean;
}): React.JSX.Element {
  const theme = useTheme();
  const muted = theme.palette.text.secondary;
  const accent = providerVisual(
    provider,
    theme.palette.mode,
    providerVersion,
    providerDigest,
  ).primary;
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
      {active
        ? <CircularProgress size={16} color="inherit" aria-label="Compacting" sx={{ color: accent }} />
        : <UnfoldLess aria-hidden sx={{ fontSize: glyphSize(16), color: muted }} />}
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

export function toolStatusTone(
  status: string,
  theme: Theme,
): string {
  if (status === "completed") return theme.palette.success.main;
  if (status === "failed") return theme.palette.error.main;
  if (status === "in_progress" || status === "interrupted") return theme.palette.warning.main;
  return theme.palette.text.disabled;
}

// A transcript image: a bounded, tappable thumbnail that opens the shared
// zoom/pan lightbox. The chunk's src is already a downscaled (~1568px) data URL
// (see attachments.ts), so it's light enough to inline here and sharp enough to
// zoom. plate={false}: chat images are screenshots/photos, not white-bg figures
// that need a frame.
export function TranscriptImage(
  { src, alt }: { src: string; alt: string },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const openTap = useReliableTouchTap<HTMLImageElement>(() => setOpen(true));
  return (
    <>
      {
        /* The pixel cap sits on this frame so the fit-content user bubble
          honours it; the image only shrinks to the frame. */
      }
      <Box
        data-transcript-image-frame="true"
        sx={{ maxWidth: MESSAGE_PREVIEW_MAX_WIDTH_PX, my: 0.5, mx: "auto" }}
      >
        <Box
          component={ProtectedImage}
          src={src}
          alt={alt}
          loading={src.startsWith("data:") || src.startsWith("blob:")
            ? undefined
            : "lazy"}
          {...openTap}
          sx={{
            maxWidth: "100%",
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
            mx: "auto",
            cursor: "zoom-in",
            touchAction: "manipulation",
          }}
        />
      </Box>
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

export function ChunkView({
  chunk,
  invert,
  streaming = false,
}: {
  chunk: ContentChunk;
  invert: boolean;
  /** This is the text the model is still producing. */
  streaming?: boolean;
}): React.JSX.Element {
  if (chunk.type === "image") {
    return <TranscriptImage src={chunk.src} alt={chunk.alt ?? ""} />;
  }
  if (chunk.type === "file") {
    return (
      <TranscriptFileChip
        name={chunk.name}
        mimeType={chunk.mimeType}
        invert={invert}
      />
    );
  }
  // Conversation prose keeps wide code and tables intact: a box diagram or a
  // many-column table wrapped to a phone column is unreadable. Each block
  // scrolls sideways only when it really overflows (`hasHorizontalScroller`
  // then gives that pan to the block; prose still drives the drawer). The
  // live row window bounds how many such scrollers the peek carries.
  return <Markdown text={chunk.text} invert={invert} streaming={streaming} />;
}

// A user-message body that collapses when it's very tall — a pasted log / big
// snippet shouldn't flood the transcript (the reported case). Community pattern
// (Claude.ai / ChatGPT / Zed, OneReach's "show more" widget, the CSS-Tricks
// fade-read-more): clamp to a max-height, fade the cut edge to the bubble
// colour, and toggle Show more / Show less. The cap is measured off the NATURAL
// content (the inner ref isn't clamped, so the clamp can never hide its own
// toggle) and re-measured on async growth (a pasted image loading). One height
// cap + a centered text toggle behaves identically on mobile + desktop.
export function CollapsibleUserBody({
  children,
  containsImage = false,
}: {
  children: React.ReactNode;
  containsImage?: boolean;
}): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const [naturalHeight, setNaturalHeight] = useState<number | null>(null);
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
      if (entry) setNaturalHeight(entry.contentRect.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // Cap from the first frame: short content is unaffected, while a long message
  // cannot flash fully expanded before the observer's initial delivery.
  // Images are already viewport-bounded thumbnails. Clamping their parent to
  // the text-only 200px cap is what made a portrait screenshot look like it
  // never expanded, so image messages keep their full preview.
  const measured = naturalHeight !== null;
  const clamp = userBubbleShouldClamp({
    measured,
    naturalHeight: naturalHeight ?? 0,
    expanded,
    containsImage,
  });
  const overflowing = measured &&
    naturalHeight > USER_BUBBLE_COLLAPSE_PX + USER_BUBBLE_COLLAPSE_BUFFER_PX;
  return (
    <>
      <Box sx={{ position: "relative" }}>
        <Box
          sx={{
            maxHeight: clamp ? USER_BUBBLE_COLLAPSE_PX : "none",
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
              "& .MuiButton-endIcon > svg": { fontSize: glyphSize(18) },
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
// primary fill). Local persistence and transport confirmation are both visible:
// the bubble dims, a white sweep runs across it, and an explicit status line
// sits below it. `failed` → red edge + Failed + retry /
// discard. Reconciled out by cmid the moment the echo Envelope lands (same
// bubble, seamless — the dim + line vanish as it confirms).
export function OptimisticUserBubble({
  sessionId,
  message,
}: {
  sessionId: string;
  message: QueuedMessage;
}): React.JSX.Element {
  const connected = useConnected();
  const appearance = pendingSyncAppearance(message.status, connected);
  const failed = appearance === "failed";
  const saving = appearance === "saving";
  const sending = appearance === "sending";
  const syncing = appearance === "syncing";
  const inFlight = saving || sending || syncing;
  const cmid = message.cmid ?? "";
  const content = attachmentDisplayParts(message.text, message.attachments);
  const returnHome = homeForOrigin(message.origin ?? "composer");
  return (
    <Stack
      alignItems="flex-end"
      spacing={0.25}
      sx={{ alignSelf: "stretch", maxWidth: "100%" }}
    >
      <Paper
        elevation={0}
        sx={(theme) => ({
          position: "relative",
          p: { xs: 1, sm: 1.25 },
          opacity: inFlight ? 0.62 : 1,
          transition: "opacity 0.2s ease",
          ...messageBubbleLayoutSx("user"),
          ...messageBubbleSurfaceSx("user", theme, failed),
        })}
      >
        {content.length === 0
          ? <Markdown text="📎 attachment" invert />
          : content.map((part, index) =>
            part.type === "text"
              ? <Markdown key={`text-${index}`} text={part.text} invert />
              : part.attachment.isImage && isLoadablePreviewUrl(part.attachment.previewUrl)
              ? (
                <TranscriptImage
                  key={`attachment-${index}-${part.attachment.id}`}
                  src={part.attachment.previewUrl as string}
                  alt={part.attachment.name}
                />
              )
              : (
                <TranscriptFileChip
                  key={`attachment-${index}-${part.attachment.id}`}
                  name={part.attachment.name}
                  mimeType={part.attachment.mimeType}
                  invert
                />
              )
          )}
        {inFlight && (
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
      {(saving || sending) && (
        <Stack
          role="status"
          aria-live="polite"
          direction="row"
          spacing={0.5}
          alignItems="center"
          sx={{ pr: 0.25 }}
        >
          <CircularProgress
            size={desktopSize(11)}
            thickness={5}
            sx={{ color: "text.secondary" }}
          />
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            {saving ? "Saving…" : "Sending…"}
          </Typography>
        </Stack>
      )}
      {syncing && (
        <Stack
          role="status"
          aria-live="polite"
          direction="row"
          spacing={0.5}
          alignItems="center"
          sx={{ pr: 0.25 }}
        >
          <CloudUpload sx={{ fontSize: glyphSize(14), color: "info.main" }} />
          <Typography variant="caption" sx={{ color: "info.main" }}>
            {connected ? "Waiting for Cowboy…" : "Waiting for connection…"}
          </Typography>
        </Stack>
      )}
      {failed && (
        <Stack direction="row" spacing={0.25} alignItems="center">
          <Typography variant="caption" sx={{ color: "error.main" }}>
            {message.failure ?? "Delivery not confirmed. Your message is kept here."}
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
          <Tooltip title={returnLabelForHome(returnHome)}>
            <IconButton
              size="small"
              aria-label={returnLabelForHome(returnHome)}
              onClick={(): void => {
                void returnFailedMessage(sessionId, cmid).catch(() => undefined);
              }}
            >
              <Undo fontSize="small" />
            </IconButton>
          </Tooltip>
          <Tooltip title="Discard">
            <IconButton
              size="small"
              aria-label="discard message"
              onClick={(): void => {
                void discardMessage(sessionId, cmid).catch(() => undefined);
              }}
            >
              <Close fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>
      )}
    </Stack>
  );
}

export function MessageBubble({
  role,
  chunks,
  streaming,
  origin,
  cmid,
  provider,
  providerVersion,
  providerDigest,
  desktop,
}: {
  role: "assistant" | "user";
  chunks: ContentChunk[];
  /** When true, append a blinking caret after the last text chunk to signal
   *  the model is still producing. */
  streaming?: boolean;
  origin?: PromptOrigin | undefined;
  cmid?: string | undefined;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  desktop: boolean;
}): React.JSX.Element | null {
  const mine = role === "user";
  const human = !mine || isHumanPrompt(origin);
  const displayChunks: ContentChunk[] = mine
    ? [...applySendImagePreviews(chunks, cmid)]
    : chunks;
  // Claude Code's "Compacting..." auto-compaction notice → purpose-built widget
  // instead of a stray one-word assistant reply. `streaming` (last item + turn
  // busy) means it's condensing right now; otherwise it's a finished record.
  if (!mine && isCompactingMessage(chunks)) {
    return (
      <CompactingWidget
        active={!!streaming}
        provider={provider}
        providerVersion={providerVersion}
        providerDigest={providerDigest}
        desktop={desktop}
      />
    );
  }
  if (!mine && isCompactionCompletion(chunks)) {
    return (
      <CompactingWidget
        active={false}
        provider={provider}
        providerVersion={providerVersion}
        providerDigest={providerDigest}
        desktop={desktop}
      />
    );
  }
  if (mine && isCompactionCommand(chunks)) {
    // The live-edge CompactingWidget is the one lifecycle surface on both
    // products. A second slash-command chip made Desktop diverge from Mobile.
    return null;
  }
  const lastChunkIdx = displayChunks.length - 1;
  const body = displayChunks.map((c, i) => (
    <Box key={i} sx={{ position: "relative" }}>
      <ChunkView chunk={c} invert={mine && human} streaming={!!streaming && i === lastChunkIdx} />
      {streaming && i === lastChunkIdx && c.type === "text" && (
        <StreamingCaret />
      )}
    </Box>
  ));
  // Non-human user-role prompts (historical Cowboy continuation/schedule, agent runtime)
  // stay on the timeline but never use the primary-filled human bubble.
  if (mine && origin && !human) {
    return (
      <PromptOriginNote
        origin={origin}
        chunks={chunks}
        provider={provider}
        providerVersion={providerVersion}
        providerDigest={providerDigest}
      />
    );
  }
  // Send and receive share a chat-card: three large corners, a tight tail on
  // the speaker-side bottom (right for you, left for the agent). Assistant
  // keeps a quiet fill so long Markdown still reads as a message.
  if (!mine) {
    return (
      <Paper
        elevation={0}
        sx={(theme) => ({
          p: { xs: 1.15, sm: 1.4 },
          ...messageBubbleLayoutSx("assistant"),
          ...messageBubbleSurfaceSx("assistant", theme),
        })}
      >
        {body}
      </Paper>
    );
  }
  return (
    <Paper
      elevation={0}
      sx={(theme) => ({
        p: { xs: 1, sm: 1.25 },
        ...messageBubbleLayoutSx("user"),
        ...messageBubbleSurfaceSx("user", theme),
      })}
    >
      <CollapsibleUserBody
        containsImage={chunks.some((chunk) => chunk.type === "image")}
      >
        {body}
      </CollapsibleUserBody>
    </Paper>
  );
}

/** Lifecycle notes (recovered, interrupted, crashed) are caption text with a
 * glyph sized from that text, so both follow the global font size together and
 * the glyph centres on the first line instead of towering over it. */
export const LIFECYCLE_ROW_SX = {
  display: "flex",
  alignItems: "flex-start",
  gap: "0.5em",
  typography: "caption",
  lineHeight: 1.5,
} as const;

// 1.5em equals the 1.5 line height, so the glyph spans the first line exactly.
export const LIFECYCLE_ICON_SX = { fontSize: "1.5em", flexShrink: 0 } as const;

export function ToolCard({
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
  const mcp = mcpIdentity(item.toolName, item.rawInput, item.title);
  const hasDetail = item.rawInput !== undefined || item.content !== undefined || mcp !== null;
  const openDetail = (): void => {
    if (hasDetail) onOpen(item.key);
  };
  const openTap = useReliableTouchTap<HTMLDivElement>(openDetail);
  const running = item.status === "in_progress" || item.status === "pending";
  // The header shows only the first line of the title — a Bash title IS the whole
  // (possibly multi-line) command, which would otherwise blow up the row.
  const fallbackTitle = mcp ? mcpHeading(mcp) : (item.title || "").split("\n")[0] || item.title;
  const [headerTitle, setHeaderTitle] = useState(fallbackTitle);
  useEffect(() => {
    setHeaderTitle(fallbackTitle);
    if (
      item.toolKind !== "execute" || !item.rawInput ||
      typeof item.rawInput !== "object"
    ) return;
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
      const summary = display.summary ||
        display.text.split("\n").find((line) => line.trim())?.trim();
      if (summary) setHeaderTitle(summary);
    });
    return () => {
      active = false;
    };
  }, [fallbackTitle, item.rawInput, item.toolKind]);
  return (
    <Paper
      elevation={0}
      sx={(theme) => ({
        alignSelf: "stretch",
        overflow: "hidden",
        // Borderless: a filled `background.paper` surface (a touch lighter than
        // the transcript bg) reads as a soft card WITHOUT a 1px outline. The
        // outlined border (theme `divider`) drew a faint violet line top+bottom
        // on every collapsed card, and a tool-heavy transcript stacked them into
        // a "ruled paper" look. The icon + status chip already mark it as a tool.
        bgcolor: "background.paper",
        // Keep the card borderless, but give the execution state a quiet wash
        // that reads in a long tool run. It preserves the existing fill-based
        // selection language and avoids a stack of purple leading rails.
        backgroundImage: `linear-gradient(105deg, ${alpha(
          toolStatusTone(item.status, theme),
          theme.palette.mode === "dark" ? 0.1 : 0.055,
        )}, transparent 48%)`,
        boxShadow: `0 1px 3px ${alpha(
          theme.palette.common.black,
          theme.palette.mode === "dark" ? 0.16 : 0.06,
        )}`,
        borderRadius: desktop ? `${DESKTOP_INSET_RADIUS}px` : 1,
        // Subtle breathing while a tool is mid-flight; nothing while
        // completed/failed (those are static states).
        animation: running ? `${pulse} 1.6s ease-in-out infinite` : undefined,
      })}
    >
      <Stack
        {...(hasDetail
          ? {
            ...openTap,
            role: "button",
            tabIndex: desktop ? -1 : 0,
            "data-haptic": "selection",
            "aria-expanded": selected,
            "aria-haspopup": "dialog" as const,
            onKeyDown: (event: React.KeyboardEvent<HTMLDivElement>): void => {
              const key = shortcutKey(event.nativeEvent);
              if (key === "Enter" || key === " ") {
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
            },
          }),
        }}
      >
        {toolIcon(mcp ? "mcp" : item.toolKind)}
        <Box sx={{ flex: 1, minWidth: 0 }}>
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
            fontFamily: !mcp && item.toolKind === "execute"
              ? "ui-monospace, SFMono-Regular, Menlo, monospace"
              : "var(--cowboy-reading-font, inherit)",
          }}
        >
          {headerTitle}
        </Typography>
        {mcp && (
          <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block" }}>
            {mcp.server} · {toolCopyText({ title: mcp.tool, toolName: item.toolName, rawInput: item.rawInput }).split("\n")[0]}
          </Typography>
        )}
        </Box>
        <ToolStatusChip status={item.status} />
        {hasDetail && (
          <ExpandMore
            fontSize="medium"
            // The rotated 24px glyph keeps ~8px of transparent box past its
            // tip, so without the pull the right inset read twice the left.
            sx={{ transform: "rotate(-90deg)", color: "text.secondary", mr: -1 }}
          />
        )}
      </Stack>
    </Paper>
  );
}

// NOTE: the agent's plan is no longer rendered inline here — it's surfaced by
// the pinned, collapsible PlanDock above the composer (src/PlanDock.tsx), so the
// task's progress stays in view without scrolling. See derive.ts `latestPlan`.

export function PermissionCard({
  item,
}: {
  item: Extract<RenderItem, { kind: "permission" }>;
}): React.JSX.Element {
  // The timeline keeps only a compact RECORD marker. The decision is taken in
  // the Conversation pane's approval dock on Desktop and in the Composer's
  // sticky PermissionOverlay on Mobile. Pending = an amber "Permission
  // requested · cmd" line; resolved = a subtle italic "Allowed/Rejected · cmd"
  // so a glaring card doesn't sit in the log forever.
  // One line either way: the full request is the card above the Composer (or
  // the Desktop dock), and the tool row above already names it. Repeating a
  // long question here showed the same text three times.
  const question = item.requestKind === "question";
  if (item.resolved) {
    const rejected = item.chosen?.toLowerCase().startsWith("reject") ?? false;
    return (
      <Stack
        direction="row"
        spacing={1}
        alignItems="center"
        sx={{ alignSelf: "stretch", minWidth: 0, color: "text.secondary", px: 0.5 }}
      >
        <Typography variant="caption" noWrap sx={{ fontStyle: "italic", minWidth: 0 }}>
          {question
            ? rejected ? "Declined" : "Answered"
            : rejected ? "Rejected" : "Allowed"}
          {!question && item.chosen ? `: ${item.chosen}` : ""} · {item.subagent ? `${item.subagent}: ` : ""}
          {item.title}
        </Typography>
      </Stack>
    );
  }
  return (
    <Stack
      direction="row"
      spacing={0.75}
      alignItems="center"
      sx={{
        alignSelf: "stretch",
        minWidth: 0,
        color: question ? "primary.main" : "warning.main",
        px: 0.5,
      }}
    >
      {question
        ? <HelpOutlineRounded sx={{ fontSize: glyphSize(18), flexShrink: 0 }} />
        : <WarningAmberRounded sx={{ fontSize: glyphSize(18), flexShrink: 0 }} />}
      <Typography variant="caption" noWrap sx={{ fontWeight: 600, minWidth: 0 }}>
        {item.subagent ? `${item.subagent}: ` : ""}
        {question ? "Waiting for your answer" : `Permission requested · ${item.title}`}
      </Typography>
    </Stack>
  );
}

/** The turn died but the worker did not: state the recovery and offer it.
 *
 *  Typing the continuation by hand was the only way to resume, every time. The
 *  button sends the same thing the reader would have typed, plus the fact only
 *  Cowboy knows — that the cut was the transport (turnRecovery.ts).
 *
 *  It also reports the failure once per card, because "this happens now and
 *  then" is not a number: the event is what will say whether this is worth
 *  automating, and for which Provider. */
export function TurnFailureRecovery({
  itemKey,
  detail,
  sessionId,
  status,
  provider,
}: {
  itemKey: string;
  detail: string | null;
  sessionId: string;
  status: Status;
  provider: string;
}): React.JSX.Element {
  const continuable = canContinueFailedTurn(status);
  useEffect(() => {
    if (reportedTurnFailures.has(itemKey)) return;
    reportedTurnFailures.add(itemKey);
    reportClientLog(
      "warn",
      "turn_failure_surfaced",
      "A turn ended in a Provider failure",
      turnFailureTelemetry(provider, detail ? rpcErrorKind(detail) : null),
    );
  }, [detail, itemKey, provider]);
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.75}
      sx={{ mt: 0.25, flexWrap: "wrap" }}
    >
      <Typography variant="caption" sx={{ color: "text.secondary" }}>
        {continuable
          ? "The session is still open where it stopped."
          : "Send a message to continue."}
      </Typography>
      {continuable && (
        <Button
          size="small"
          variant="outlined"
          color="inherit"
          data-turn-failure-continue
          onClick={(): void => {
            void submitPrompt(sessionId, TURN_CONTINUATION_PROMPT);
          }}
          sx={{ minHeight: 28, py: 0.1, px: 1, textTransform: "none" }}
        >
          Continue
        </Button>
      )}
    </Stack>
  );
}

// One report per card, not per render: the transcript re-renders on every
// scroll threshold and the count would be meaningless.
export const reportedTurnFailures = new Set<string>();

// `memo`'d: with `derive` memoized upstream, each item keeps a stable identity
// across renders that don't change the timeline (e.g. scroll-threshold or
// permission-sheet state). Without this, any such re-render would re-run the
// markdown parser + syntax highlighter for every visible row — the source of
// the scroll jitter on mobile.
export const ItemView = memo(function ItemView({
  item,
  streaming,
  sessionId,
  status,
  provider,
  providerVersion,
  providerDigest,
  desktop,
  selectedToolKey,
  onOpenTool,
  tasks,
  stops,
}: {
  item: RenderItem;
  sessionId: string;
  /** The session's live status — a failed turn is only continuable while its
   *  worker is still there (turnRecovery.ts). */
  status: Status;
  /** True when this item is the last assistant chunk-bearing item and the
   *  session is still busy. Adds a blinking caret / dots accordingly. */
  streaming?: boolean;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  desktop: boolean;
  selectedToolKey: string | null;
  onOpenTool: (key: string) => void;
  /** Background-task summaries for subagent and Call anchor rows. */
  tasks?: Pick<BackgroundTasks, "byKey" | "callsObservedAt" | "unresolvedCalls"> | undefined;
  stops?: ReadonlyMap<string, import("./useBackgroundTasks").StopState> | undefined;
}): React.JSX.Element | null {
  switch (item.kind) {
    case "message": {
      const bubble = (
        <MessageBubble
          role={item.role}
          chunks={item.chunks}
          streaming={!!streaming && item.role === "assistant"}
          origin={item.origin}
          cmid={item.cmid}
          provider={provider}
          providerVersion={providerVersion}
          providerDigest={providerDigest}
          desktop={desktop}
        />
      );
      return bubble;
    }
    case "thought":
      return (
        <Box
          sx={{
            color: "text.secondary",
            alignSelf: "stretch",
            maxWidth: "100%",
            px: 0,
          }}
        >
          <Box sx={{ fontSize: "0.84rem", flex: 1, minWidth: 0 }}>
            <ProviderThoughtSteps
              sections={item.sections}
              streaming={!!streaming}
              provider={provider}
              providerVersion={providerVersion}
              providerDigest={providerDigest}
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
    case "subagent": {
      const task = tasks?.byKey.get(subagentKey(item.subagent.id));
      return task
        ? <TaskRow task={task} sessionId={sessionId} desktop={desktop} stop={stops?.get(task.key)} />
        : null;
    }
    case "call": {
      const task = tasks?.byKey.get(callKey(item.callId));
      if (task) return <TaskRow task={task} sessionId={sessionId} desktop={desktop} stop={stops?.get(task.key)} />;
      // Until the ledger answers, keep the row's geometry; only a call it says
      // is gone (dismissed or cleared) leaves no row behind.
      return tasks?.callsObservedAt === null || tasks?.unresolvedCalls.has(item.callId)
        ? <PendingCallRow desktop={desktop} />
        : null;
    }
    case "lifecycle": {
      if (item.recovered) {
        return (
          <Box sx={{ ...LIFECYCLE_ROW_SX, color: "text.secondary" }}>
            <CheckCircleOutline sx={LIFECYCLE_ICON_SX} />
            <Box
              component="details"
              data-runtime-recovered
              sx={{
                minWidth: 0,
                // The disclosure is ours, sized with the caption: the native
                // marker ignored the font scale and the theme font.
                "& > summary": {
                  cursor: "pointer",
                  listStyle: "none",
                  display: "inline-flex",
                  alignItems: "center",
                  gap: "0.25em",
                  "&::-webkit-details-marker": { display: "none" },
                },
                "& > summary > svg": { fontSize: "1.25em", transition: "transform 120ms ease" },
                "&[open] > summary > svg": { transform: "rotate(90deg)" },
              }}
            >
              <Box component="summary">
                Session recovered
                <ChevronRight />
              </Box>
              <Box sx={{ mt: 0.5, overflowWrap: "anywhere", whiteSpace: "pre-wrap", maxHeight: "30vh", overflow: "auto" }}>
                Earlier error: {item.detail ?? "The session stopped unexpectedly."}
              <ScrollEdges /></Box>
            </Box>
          </Box>
        );
      }
      // The interrupted marker (a turn cut off by a daemon restart) is a durable,
      // amber record that stays in the log after the session resumes; crashes are
      // red. Clean exits are session chrome, not a transcript event.
      const label = item.turnFailure && item.detail
        ? prettifyCrashDetail(item.detail)
        : transcriptLifecycleLabel(item.status, item.detail, prettifyCrashDetail);
      if (!label) return null;
      const interrupted = item.status === "interrupted";
      const color = item.status === "crashed"
        ? "error.main"
        : interrupted
        ? "warning.main"
        : "text.secondary";
      return (
        <Box sx={{ ...LIFECYCLE_ROW_SX, color }}>
          {interrupted
            ? <WarningAmberRounded sx={LIFECYCLE_ICON_SX} />
            : <ErrorOutline sx={LIFECYCLE_ICON_SX} />}
          <Box sx={{ minWidth: 0 }}>
            <Box component="span" sx={{ fontWeight: interrupted ? 600 : 400 }}>
              {label}
            </Box>
            {item.turnFailure && (
              <TurnFailureRecovery
                itemKey={item.key}
                detail={item.detail}
                sessionId={sessionId}
                status={status}
                provider={provider}
              />
            )}
            {item.turnFailure && item.detail && (
              <ErrorDetails detail={item.detail} />
            )}
          </Box>
        </Box>
      );
    }
    case "forked":
      // Filtered out of the Transcript; provenance lives in Session Settings.
      return null;
    case "cleared":
      // A compact context boundary, not a full-width divider. The old pair of
      // rules visually split an otherwise empty transcript and competed with
      // the Composer. This quiet badge still records the durable reset while
      // reading naturally whether older history exists above it or not.
      return (
        <Box
          data-transcript-context-boundary
          role="status"
          aria-label="Conversation cleared. New context started."
          sx={(theme) => ({
            alignSelf: "center",
            display: "inline-flex",
            alignItems: "center",
            gap: 0.625,
            my: 0.75,
            px: 1,
            py: 0.5,
            borderRadius: 999,
            color: "text.secondary",
            bgcolor: alpha(theme.palette.background.paper, 0.66),
            border: `1px solid ${alpha(theme.palette.divider, 0.72)}`,
            boxShadow: `0 4px 14px ${alpha(theme.palette.common.black, 0.045)}`,
            backdropFilter: "blur(10px) saturate(120%)",
            WebkitBackdropFilter: "blur(10px) saturate(120%)",
          })}
        >
          <Box
            sx={(theme) => ({
              width: 22,
              height: 22,
              borderRadius: "50%",
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              color: "primary.main",
              bgcolor: alpha(theme.palette.primary.main, 0.09),
            })}
          >
            <CleaningServices sx={{ fontSize: glyphSize(13) }} />
          </Box>
          <Typography
            variant="caption"
            sx={{ fontWeight: 650, letterSpacing: 0.1 }}
          >
            New conversation
          </Typography>
        </Box>
      );
    case "compaction": {
      if (item.status === "failed" || item.status === "cancelled") {
        const failed = item.status === "failed";
        return (
          <Box sx={{ ...LIFECYCLE_ROW_SX, color: failed ? "error.main" : "warning.main" }}>
            {failed
              ? <ErrorOutline sx={LIFECYCLE_ICON_SX} />
              : <WarningAmberRounded sx={LIFECYCLE_ICON_SX} />}
            <Box sx={{ minWidth: 0 }}>
              <Box component="span">
                {failed ? "Context compaction failed" : "Context compaction cancelled"}
              </Box>
              {item.error && <ErrorDetails detail={item.error} />}
            </Box>
          </Box>
        );
      }
      return (
        <Box sx={{ display: "flex", flexDirection: "column", alignItems: "flex-start" }}>
          <CompactingWidget
            active={item.status === "in_progress" && status === "busy"}
            provider={provider}
            providerVersion={providerVersion}
            providerDigest={providerDigest}
            desktop={desktop}
          />
          {item.summary.trim() && (
            <Box
              component="details"
              sx={{ color: "text.secondary", fontSize: "0.75rem", maxWidth: "100%" }}
            >
              <Box component="summary" sx={{ cursor: "pointer" }}>Retained summary</Box>
              <Box
                component="div"
                sx={{
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                  mt: 0.5,
                  maxHeight: "40vh",
                  overflow: "auto",
                }}
              >
                {item.summary.trim()}
              <ScrollEdges /></Box>
            </Box>
          )}
        </Box>
      );
    }
    case "notice":
      return <NoticeRow item={item} sessionId={sessionId} />;
  }
});

export function ErrorDetails({ detail }: { detail: string }): React.JSX.Element {
  return (
    <Box component="details" sx={{ mt: 0.5, color: "text.secondary", fontSize: "0.75rem" }}>
      <Box component="summary" sx={{ cursor: "pointer" }}>Details</Box>
      <Box component="pre" sx={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", m: 0, mt: 0.5, maxHeight: "20vh", overflow: "auto" }}>
        {detail}
      <ScrollEdges /></Box>
    </Box>
  );
}
