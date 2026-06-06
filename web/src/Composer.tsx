import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Button,
  CircularProgress,
  Collapse,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  IconButton,
  List,
  Menu,
  MenuItem,
  Paper,
  Popover,
  Skeleton,
  Stack,
  TextField,
  Tooltip,
  Typography,
  useMediaQuery,
  useTheme,
} from "@mui/material";
import {
  AlternateEmail,
  AttachFile,
  Bolt,
  ChevronRight,
  Close,
  DragIndicator,
  EditNoteOutlined,
  EditOutlined,
  ExpandMore,
  InsertDriveFileOutlined,
  Send,
  Stop,
  Tune,
  VerticalAlignBottom,
} from "@mui/icons-material";
import { ComposerEditor, type ComposerEditorHandle } from "./ComposerEditor";
import { ComposerTextarea, useTouchComposer } from "./ComposerTextarea";
import { PlanDock } from "./PlanDock";
import { latestPlan } from "./derive";
import { useVimSetting } from "./vimSetting";
import { type Attachment, filesToAttachments } from "./attachments";
import {
  activateAllDrafts,
  activateDraft,
  addDraft,
  clearDrafts,
  clearQueue,
  editDraft,
  editQueued,
  forcePushQueued,
  type QueuedMessage,
  queuedToDraft,
  removeDraft,
  removeQueued,
  reorderDrafts,
  reorderQueue,
  requestSendQueued,
  send,
  setQueueEditing,
  submitPrompt,
  useStore,
} from "./store";
import { useSortable } from "./useSortable";
import { getDraft, setDraft } from "./draftStore";
import { useNavbarAtBottom } from "./navbarSettings";
import { useReadingSettings } from "./readingSettings";
import { requestStickToBottom, setSticky, useSticky } from "./stickyStore";
import { originLabel } from "./protocol";
import type {
  AcpUpdate,
  AvailableCommand,
  ConfigOption,
  Envelope,
  SessionMeta,
  Status,
} from "./protocol";
import { BottomSheet } from "./_shell";

// Cmd/Ctrl + Enter = send. Plain Enter = newline.
//
// Why this way (not the reverse): pasting multi-line code / prompts is a
// daily action; making plain Enter send would shred any pasted snippet that
// contains a newline. ChatGPT/Claude.ai/Cursor all default to "Enter =
// newline + Cmd-Enter sends" for the same reason. Touch keyboards inherit
// the same model — their Enter key inserts a newline and the user taps the
// send button.
//
// Layout: mobile-first. Composer sits at the bottom of the viewport with a
// safe-area inset. Action row (slash-command / @-reference triggers + the
// agent-advertised config chips for mode / model / effort) sits BELOW the
// textarea so the textarea always stays wide and tap-able. The row is
// horizontally scrollable on narrow viewports so any number of dropdowns
// doesn't force a wrap.

// Toolbar control sizing keys off POINTER TYPE, not viewport width. A desktop
// with a narrow window is still a mouse (precise) and wants dense controls; any
// touch device wants the ≥40px tap target (ui.md §7 "mobile never small"). The
// old `{ xs: 40, lg: 36 }` viewport-breakpoint sizing got this wrong twice: a
// sub-`lg` desktop window fell back to the chunky 40px touch size, and an iPad
// in a wide (≥`lg`) layout got the cramped 36px desktop size. `pointer: coarse`
// is the right axis — it tracks the input device, not the window. Desktop is a
// dense 32px; coarse pointers bump every control to 40.
const TOOLBAR_ICON_BTN = {
  width: 32,
  height: 32,
  flexShrink: 0,
  "@media (pointer: coarse)": { width: 40, height: 40 },
} as const;
const TOOLBAR_MIN_H = {
  minHeight: 34,
  "@media (pointer: coarse)": { minHeight: 40 },
} as const;

export function Composer({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  // Draft state is seeded from the per-session draft store and persisted back to
  // it (see the effect below). The Composer is remounted per session (key in
  // App), so these initializers read the right session's draft on mount and a
  // session switch never carries a draft across.
  const [text, setText] = useState<string>(() => getDraft(sessionId).text);
  // CodeMirror is UNCONTROLLED: it's seeded with the session's draft once (this
  // stable ref, captured at mount — the Composer remounts per session via the
  // App-level key) and then OWNS its document. We deliberately never feed `text`
  // back as the editor's `value` on every keystroke. Doing so re-applied the doc
  // on each render, which on iOS (a) left the just-sent text on screen after
  // clear() and (b) bounced the caret — a typed comma landing after the cursor.
  // onChange keeps `text` in sync for send / sendable / draft; clear() empties
  // the doc imperatively on submit.
  const initialDraftText = useRef<string>(getDraft(sessionId).text);
  // Staged image / file attachments — previewed above the editor and sent as
  // ACP content blocks alongside the text (see attachments.ts). Cleared on send.
  const [attachments, setAttachments] = useState<Attachment[]>(
    () => getDraft(sessionId).attachments,
  );
  // Persist the in-progress draft per session so switching away and back
  // restores it, and so it never bleeds into another session. Runs on mount too
  // (idempotent re-write of the seed); on submit, text/attachments go empty and
  // setDraft drops the entry.
  useEffect(() => {
    setDraft(sessionId, { text, attachments });
  }, [sessionId, text, attachments]);
  const editorRef = useRef<ComposerEditorHandle>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { configOptions, drafts, queues, sessions, timelines } = useStore();
  const queue = queues.get(sessionId) ?? [];
  const draftList = drafts.get(sessionId) ?? [];
  // The agent's current plan, pinned above the queue as a collapsible dock so
  // task progress stays in view without scrolling the transcript. null = no plan.
  const plan = useMemo(() => latestPlan(timelines.get(sessionId) ?? []), [timelines, sessionId]);
  // Manual dismiss: keyed on the plan's step list so it stays gone as the agent
  // updates statuses, but a genuinely new plan (different steps) reappears.
  const [dismissedPlanKey, setDismissedPlanKey] = useState<string | null>(null);
  // Show the plan unless (a) the user dismissed this exact plan, or (b) it's
  // fully complete AND the user has already moved on to a new turn — ACP never
  // signals "plan done", so a finished plan would otherwise linger forever.
  const showPlan =
    plan !== null &&
    plan.key !== dismissedPlanKey &&
    !(plan.supersededByUserTurn && plan.entries.every((e) => e.status === "completed"));
  // The active session's metadata, surfaced read-only inside the options
  // sheet (mobile's "session settings" popup). Desktop shows the same facts
  // in the always-visible sidebar, so the sheet — and this lookup — only
  // matters on the compact tier.
  const session = sessions.find((s) => s.id === sessionId);
  const theme = useTheme();
  // Touch tier collapses the agent config into a single Tune button — tapping
  // it opens a BottomSheet with the session info + every config option in one
  // place. Inspired by ChatGPT / DeepSeek / Gemini: chips wrap awkwardly on
  // iPad portrait (820px) and are completely unreadable on a 390px iPhone, so
  // the sheet pattern wins on every sub-desktop viewport. Desktop keeps the
  // inline chip row — there's room.
  const compact = useMediaQuery(theme.breakpoints.down("lg"));
  const [sheetOpen, setSheetOpen] = useState(false);
  // Stopping a running turn is confirmed through a modal (Enter confirms, Esc
  // dismisses) — clicking Stop or pressing Esc in the editor opens it, rather
  // than cancelling on a single stray click/keypress.
  const [cancelOpen, setCancelOpen] = useState(false);
  // "Stick to bottom" (auto-scroll) state for this session — owned by the
  // Transcript's scroll engine, surfaced here as a persistent toggle. Active =
  // following the latest message; tap while inactive scrolls to the bottom and
  // resumes following, tap while active stops following.
  const sticky = useSticky(sessionId);
  // When the navbar sits at the bottom it owns the home-indicator safe area and
  // renders BELOW the composer, so the composer must drop its own bottom inset
  // (otherwise a double gap opens above the bar).
  const navbarAtBottom = useNavbarAtBottom();
  // The composer's horizontal gutter follows the reading `padding` so it lines
  // up with the transcript content above it (which uses the same value). Floored
  // at the safe-area inset so a small padding can't tuck the action-row buttons
  // under the landscape notch / rounded corner.
  const { padding } = useReadingSettings();

  const busy = status === "busy";
  const starting = status === "starting";
  const dead = status === "exited" || status === "crashed";
  // A dead session is still sendable: sending resumes it (the daemon revives
  // the agent via session/load — see supervisor.rs). Matches Zed, where a
  // thread is never permanently unusable just because its agent process ended.
  // An attachment-only prompt (e.g. just a pasted screenshot) is also sendable.
  const sendable = !!text.trim() || attachments.length > 0;

  // Read picked / pasted files into ACP content blocks and stage them. Async
  // (FileReader), so previews appear once each file is encoded; unreadable
  // files are silently dropped (filesToAttachments filters them).
  function addFiles(files: File[]): void {
    if (files.length === 0) return;
    void filesToAttachments(files).then((added) => {
      if (added.length > 0) setAttachments((prev) => [...prev, ...added]);
    });
  }

  function removeAttachment(id: string): void {
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  }

  // Pull the agent-advertised options for this session, if known. Sorted in
  // a fixed display order so dropdowns don't flicker between
  // config_option_update notifications.
  const options = useMemo(() => {
    const raw = configOptions.get(sessionId) ?? [];
    const order = ["mode", "model", "effort"];
    return [...raw].sort((a, b) => {
      const ai = order.indexOf(a.id);
      const bi = order.indexOf(b.id);
      if (ai === -1 && bi === -1) return a.id.localeCompare(b.id);
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
  }, [configOptions, sessionId]);
  // `starting` is the obvious case; we also keep the skeleton on for the
  // brief window after status flips to `running` but before the agent's
  // first `config_option_update` arrives (otherwise the action row pops
  // empty for ~1 frame and then re-flows when the chips appear).
  const showSkeleton = !dead && options.length === 0 && (starting || status === "running");

  // Slash skills + `@` file references are handled inside the editor now, via
  // CodeMirror autocomplete (see ComposerEditor + composerCompletions): no more
  // Popper pickers or caret/regex bookkeeping here. The editor reads the
  // agent-advertised `/` commands through a thunk; `@` files come from the
  // daemon's `/api/sessions/{id}/files` search.
  const availableCommands = useMemo(
    () => latestAvailableCommands(timelines.get(sessionId) ?? []),
    [timelines, sessionId],
  );

  // Vim is opt-in and desktop-only — ComposerEditor gates the actual
  // `@replit/codemirror-vim` load on a precise-pointer device, so touch never
  // pays for it. The reactive setting is flipped by the Settings toggle.
  const vim = useVimSetting();
  // Touch → native textarea (correct IME); desktop → CodeMirror (vim + inline
  // completion). See ComposerTextarea for the why.
  const touchInput = useTouchComposer();

  function submit(): void {
    if (!sendable) return;
    const trimmed = text.trimEnd();
    // The daemon decides: dispatch straight through when the session can take a
    // turn now, else stack the prompt on the (server-owned) queue.
    submitPrompt(sessionId, trimmed, attachments);
    // Clear the CodeMirror document imperatively, not just via `value=""`: the
    // editor's 200ms typing latch defers prop-driven clears when you submit
    // right after typing, leaving the sent text lingering. See clear() in
    // ComposerEditor. setText("") then keeps React state in sync (idempotent —
    // value now matches the empty doc, so no second dispatch).
    editorRef.current?.clear();
    setText("");
    setAttachments([]);
  }

  // Park the composer's content as a draft (the Draft button) and clear the
  // input. Drafts persist and are activated later from the Drafts panel.
  function saveDraft(): void {
    if (!sendable) return;
    addDraft(sessionId, text.trimEnd(), attachments);
    editorRef.current?.clear();
    setText("");
    setAttachments([]);
  }

  return (
    <Box
      sx={{
        // Side gutter = the reading `padding` (so the composer lines up with the
        // transcript content above), but floored at the device safe-area inset:
        // in landscape the non-zero left/right insets push the action row's
        // far-edge buttons (slash / send) clear of the notch + rounded corner.
        // Bottom: the home-indicator inset minus 20px so the action row sits
        // tight (~14px on a home-bar iPhone instead of the full ~34px) — the
        // buttons reach a bit into the indicator zone, which is fine for taps.
        // Floored to 2px on devices without a home bar.
        pt: { xs: 1, sm: 1.5 },
        // Bottom inset only when the composer is the bottom-most element. With
        // the navbar at the bottom it sits below us and owns the home-indicator
        // inset, so we drop to a plain gap.
        pb: navbarAtBottom
          ? 1
          : { xs: "max(calc(env(safe-area-inset-bottom) - 20px), 2px)", sm: 1.5 },
        pl: `max(env(safe-area-inset-left), ${padding}px)`,
        pr: `max(env(safe-area-inset-right), ${padding}px)`,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        position: "relative", // anchor for Popper portal placement
      }}
    >
      {/* Agent plan (very top): a pinned, collapsible progress summary so the
          task's plan stays visible above the queue without scrolling. Hidden
          when there's no plan, when dismissed, or when a finished plan has been
          superseded by a new turn (see showPlan). */}
      {showPlan && plan && (
        <PlanDock entries={plan.entries} onDismiss={(): void => setDismissedPlanKey(plan.key)} />
      )}
      {/* Queued prompts (top): while the agent is busy, messages stack here and
          drain one per turn-end. Hidden when empty. */}
      {queue.length > 0 && (
        <PendingPanel
          kind="queued"
          sessionId={sessionId}
          items={queue}
          status={status}
          commands={(): AvailableCommand[] => availableCommands}
        />
      )}
      {/* Drafts (below the queue, above the input): parked messages the user
          holds and activates on demand. Persisted across reloads. */}
      {draftList.length > 0 && (
        <PendingPanel
          kind="draft"
          sessionId={sessionId}
          items={draftList}
          status={status}
          commands={(): AvailableCommand[] => availableCommands}
        />
      )}
      {/* Staged attachments (image thumbnails / file chips) sit above the editor
          so they read as "what will be sent with this message". */}
      {attachments.length > 0 && (
        <AttachmentPreviews attachments={attachments} onRemove={removeAttachment} />
      )}
      {/* Hidden multi-file picker driven by the paperclip button. `accept` is
          left open so any file type can be attached (images embed inline, other
          files ride as ACP resource blocks — see attachments.ts). */}
      <input
        ref={fileInputRef}
        type="file"
        multiple
        hidden
        onChange={(e): void => {
          addFiles(Array.from(e.target.files ?? []));
          // Reset so picking the same file twice in a row still fires change.
          e.target.value = "";
        }}
      />
      {/* Input tier: native textarea on touch (CodeMirror's contenteditable
          strands IME pinyin on iOS — see ComposerTextarea), CodeMirror on
          desktop (vim + live @/​/ completion). Same ComposerEditorHandle ref. */}
      {touchInput ? (
        <ComposerTextarea
          ref={editorRef}
          // Controlled by `text` (a native textarea handles IME under control).
          value={text}
          onChange={setText}
          onSubmit={submit}
          sessionId={sessionId}
          commands={(): AvailableCommand[] => availableCommands}
          placeholder={dead ? "Send to resume this session…" : "Message the agent…"}
          onPasteFiles={addFiles}
          onEscape={(): boolean => {
            if (busy) {
              setCancelOpen(true);
              return true;
            }
            return false;
          }}
        />
      ) : (
        <ComposerEditor
          ref={editorRef}
          // Stable seed only (uncontrolled — see initialDraftText). NOT `text`.
          value={initialDraftText.current}
          onChange={setText}
          onSubmit={submit}
          sessionId={sessionId}
          commands={(): AvailableCommand[] => availableCommands}
          placeholder={dead ? "Send to resume this session…" : "Message the agent…"}
          vim={vim}
          onPasteFiles={addFiles}
          onEscape={(): boolean => {
            // Esc cancels a running turn (via the confirm modal), but only when a
            // turn is actually in flight — otherwise leave Esc to the editor. In
            // vim, ComposerEditor only calls this once we're already in normal
            // mode, so insert-mode Esc still just exits to normal.
            if (busy) {
              setCancelOpen(true);
              return true;
            }
            return false;
          }}
        />
      )}
      {/* Action row below the input: slash-command / @-reference triggers on
          the left, then the agent config (inline chips on desktop, the bottom
          sheet on touch), then the send button. Buttons are 40px on touch so
          the side safe-area floor keeps them off the iPhone corner radius. */}
      <Stack direction="row" alignItems="center" spacing={0.5} sx={{ mt: 0.75, ...TOOLBAR_MIN_H }}>
        <Tooltip title="Slash command / skill">
          <span>
            <IconButton
              aria-label="slash command"
              disabled={dead}
              sx={TOOLBAR_ICON_BTN}
              onClick={(): void => editorRef.current?.insertTrigger("/")}
            >
              {/* rem, NOT px: the global font scale (useGlobalFontScale) grows
                  the app via the root font-size, so the sibling SvgIcons (rem)
                  scale but a px glyph wouldn't. 1.375rem = the old 22px at 100%,
                  tracking the ~1.5rem medium icons next to it. */}
              <Box component="span" sx={{ fontSize: "1.375rem", fontWeight: 700, lineHeight: 1 }}>
                /
              </Box>
            </IconButton>
          </span>
        </Tooltip>
        <Tooltip title="Reference a file (@)">
          <span>
            <IconButton
              aria-label="reference a file"
              disabled={dead}
              sx={TOOLBAR_ICON_BTN}
              onClick={(): void => editorRef.current?.insertTrigger("@")}
            >
              <AlternateEmail />
            </IconButton>
          </span>
        </Tooltip>
        <Tooltip title="Attach image or file">
          <span>
            <IconButton
              aria-label="attach image or file"
              disabled={dead}
              sx={TOOLBAR_ICON_BTN}
              onClick={(): void => fileInputRef.current?.click()}
            >
              <AttachFile />
            </IconButton>
          </span>
        </Tooltip>
        {compact ? (
          <Tooltip title="Options">
            <span>
              <IconButton
                aria-label="options"
                disabled={dead}
                sx={TOOLBAR_ICON_BTN}
                onClick={(): void => setSheetOpen(true)}
              >
                <Tune />
              </IconButton>
            </span>
          </Tooltip>
        ) : (
          <Stack
            direction="row"
            alignItems="center"
            spacing={0.5}
            sx={{
              minWidth: 0,
              overflowX: "auto",
              scrollbarWidth: "thin",
              "&::-webkit-scrollbar": { height: 6 },
            }}
          >
            {showSkeleton ? (
              <ConfigChipSkeletons />
            ) : (
              options.map((opt) => (
                <ConfigOptionChip
                  key={opt.id}
                  option={opt}
                  disabled={dead}
                  onSelect={(value): void =>
                    send({
                      type: "set_config_option",
                      session_id: sessionId,
                      config_id: opt.id,
                      value,
                    })
                  }
                />
              ))
            )}
          </Stack>
        )}

        {/* Sticky / auto-scroll toggle — rightmost of the left utility group,
            sitting just before the gap that pushes Send to the far edge. Default
            ON. Active = primary; inactive = muted. Tap while inactive → scroll to
            bottom + follow again; tap while active → stop following. The
            Transcript owns the actual scrolling (stickyStore). */}
        {/* Hover-only tooltip: on touch a tap focuses the button, and the
            focus/touch listeners would pop the "Auto-scroll: on" bubble every
            time you toggle — noise on the most-tapped control. Disable both so
            the tooltip is desktop-hover only; the button still toggles, and the
            aria-label keeps it labelled for assistive tech. */}
        <Tooltip
          title={sticky ? "Auto-scroll: on" : "Auto-scroll: off — tap to follow"}
          disableFocusListener
          disableTouchListener
        >
          <IconButton
            aria-label={sticky ? "auto-scroll on" : "auto-scroll off"}
            color={sticky ? "primary" : "default"}
            sx={TOOLBAR_ICON_BTN}
            onClick={(): void =>
              sticky
                ? setSticky(sessionId, false)
                : requestStickToBottom(sessionId)}
          >
            <VerticalAlignBottom />
          </IconButton>
        </Tooltip>

        <Box sx={{ flex: 1 }} />

        {!compact && (
          <Typography
            variant="caption"
            color="text.disabled"
            sx={{ whiteSpace: "nowrap", fontSize: 11, flexShrink: 0, mr: 0.5 }}
          >
            ⌘/Ctrl + Enter = {busy || starting ? "queue" : "send"}
          </Typography>
        )}

        {/* Draft: park the current message in the Drafts panel (persisted) to
            send later, instead of sending/queuing now. Shown only when there's
            something to save. */}
        {sendable && (
          <Tooltip title="Save as draft">
            <IconButton
              aria-label="save as draft"
              sx={TOOLBAR_ICON_BTN}
              onClick={saveDraft}
            >
              <EditNoteOutlined />
            </IconButton>
          </Tooltip>
        )}

        {/* Busy: the agent owns the turn, so the primary button is Stop. A
            secondary "queue" button appears once there's text to stack, so the
            enqueue affordance is visible (not just ⌘/Ctrl+Enter). Idle: a single
            Send button, the unchanged fast path. */}
        {busy || starting ? (
          <>
            {sendable && (
              <Tooltip title="Queue (⌘/Ctrl + Enter)">
                <span>
                  <IconButton
                    color="primary"
                    aria-label="queue message"
                    sx={TOOLBAR_ICON_BTN}
                    onClick={submit}
                  >
                    <Send />
                  </IconButton>
                </span>
              </Tooltip>
            )}
            {busy && (
              <Tooltip title="Stop">
                <IconButton
                  color="error"
                  aria-label="cancel"
                  sx={TOOLBAR_ICON_BTN}
                  onClick={(): void => setCancelOpen(true)}
                >
                  <Stop />
                </IconButton>
              </Tooltip>
            )}
          </>
        ) : (
          <Tooltip title="Send (⌘/Ctrl + Enter)">
            <span>
              <IconButton
                color="primary"
                aria-label="send"
                disabled={!sendable}
                sx={TOOLBAR_ICON_BTN}
                onClick={submit}
              >
                <Send />
              </IconButton>
            </span>
          </Tooltip>
        )}
      </Stack>
      {compact && (
        <ComposerSheet
          open={sheetOpen}
          onClose={(): void => setSheetOpen(false)}
          session={session}
          options={options}
          loading={showSkeleton}
          dead={dead}
          onSelectOption={(configId, value): void =>
            send({
              type: "set_config_option",
              session_id: sessionId,
              config_id: configId,
              value,
            })
          }
        />
      )}
      {/* Confirm before stopping a running turn. The Stop button is destructive
          (the in-flight turn ends), so a single click/Esc shouldn't trigger it.
          The Stop action is autoFocused so Enter confirms; Esc dismisses (MUI
          Dialog default), so a stray second Esc backs out rather than cancelling. */}
      <Dialog
        open={cancelOpen}
        onClose={(): void => setCancelOpen(false)}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>Stop the running turn?</DialogTitle>
        <DialogContent>
          <DialogContentText>
            The agent is still working. Stopping ends the current turn; whatever
            it produced so far stays in the transcript.
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button
            color="inherit"
            onClick={(): void => setCancelOpen(false)}
            sx={{ textTransform: "none" }}
          >
            Keep running
          </Button>
          <Button
            color="error"
            variant="contained"
            autoFocus
            onClick={(): void => {
              send({ type: "cancel", session_id: sessionId });
              setCancelOpen(false);
            }}
            sx={{ textTransform: "none" }}
          >
            Stop
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}

// Staged-attachment strip shown above the editor. Image attachments render as
// capped thumbnails; other files as a labelled chip. Each carries a remove
// button. Horizontally scrollable so a handful of attachments never wraps the
// composer or pushes the editor down (the editor must stay reachable on a
// phone). The strip scrolls inside the bar — not flush to the device bottom
// edge — so the iOS bottom-edge gesture doesn't eat the scroll (ui.md §7).
function AttachmentPreviews({
  attachments,
  onRemove,
}: {
  attachments: Attachment[];
  onRemove: (id: string) => void;
}): React.JSX.Element {
  // Thumbnails whose <img> failed to paint — fall them back to the named file
  // chip. On iOS the picker can hand back a HEIC the canvas couldn't rasterize
  // (encodeImage returned null), so the previewUrl is a `data:image/heic` URL
  // Safari won't render in an <img>: it loaded "successfully" as empty, leaving
  // a blank box. onError catches the decode failure so the user still sees the
  // attachment (a chip) instead of nothing.
  const [failedIds, setFailedIds] = useState<ReadonlySet<string>>(new Set());
  return (
    <Stack
      direction="row"
      spacing={1}
      sx={{
        mb: 1,
        pb: 0.5,
        overflowX: "auto",
        scrollbarWidth: "thin",
        "&::-webkit-scrollbar": { height: 6 },
        // iOS repaint fix, same root cause as the editor (cmTheme): inside the
        // position:fixed body WebKit may not paint freshly-inserted nodes —
        // here the thumbnail that appears after returning from the native photo
        // picker. Promoting the strip to its own compositing layer forces the
        // paint, so the preview shows up immediately.
        transform: "translateZ(0)",
      }}
    >
      {attachments.map((a) => (
        <Box key={a.id} sx={{ position: "relative", flexShrink: 0 }}>
          {a.isImage && a.previewUrl && !failedIds.has(a.id) ? (
            <Box
              component="img"
              src={a.previewUrl}
              alt={a.name}
              onError={(): void =>
                setFailedIds((prev) => new Set(prev).add(a.id))}
              sx={{
                width: 56,
                height: 56,
                objectFit: "cover",
                borderRadius: 1,
                border: 1,
                borderColor: "divider",
                display: "block",
              }}
            />
          ) : (
            <Stack
              direction="row"
              spacing={0.75}
              alignItems="center"
              sx={{
                height: 56,
                maxWidth: 180,
                px: 1,
                borderRadius: 1,
                border: 1,
                borderColor: "divider",
                bgcolor: "action.hover",
              }}
            >
              <InsertDriveFileOutlined fontSize="small" sx={{ color: "text.secondary", flexShrink: 0 }} />
              <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
                {a.name}
              </Typography>
            </Stack>
          )}
          <IconButton
            aria-label={`remove ${a.name}`}
            size="small"
            onClick={(): void => onRemove(a.id)}
            sx={{
              position: "absolute",
              top: -8,
              right: -8,
              width: 22,
              height: 22,
              bgcolor: "background.paper",
              border: 1,
              borderColor: "divider",
              "&:hover": { bgcolor: "action.hover" },
            }}
          >
            <Close sx={{ fontSize: 14 }} />
          </IconButton>
        </Box>
      ))}
    </Stack>
  );
}

// Compact, read-only attachment summary for a queued prompt row — a paperclip
// glyph + count, so a queued message that carries images/files reads as such
// without re-rendering full thumbnails in the cramped queue list.
function QueuedAttachmentChips({
  attachments,
}: {
  attachments: Attachment[];
}): React.JSX.Element {
  return (
    <Stack direction="row" spacing={0.5} alignItems="center" sx={{ color: "text.secondary", mt: 0.25 }}>
      <AttachFile sx={{ fontSize: 14 }} />
      <Typography variant="caption">
        {attachments.length} attachment{attachments.length === 1 ? "" : "s"}
      </Typography>
    </Stack>
  );
}

// The Zed-style staging panel above the editor — one component for two kinds:
//   - "queued": prompts the busy agent can't take yet, auto-drained one per turn.
//   - "draft":  parked messages the user holds; activated (sent/queued) on demand.
// Collapsible header ("N Queued Messages" / "N Drafts" + Clear All, plus Send all
// for drafts) over a scroll-capped list of rows. Drafts sit BELOW the queue and
// above the composer (see the Composer render).
function PendingPanel({
  kind,
  sessionId,
  items,
  status,
  commands,
}: {
  kind: "queued" | "draft";
  sessionId: string;
  items: QueuedMessage[];
  /** Session status — drives Send-now vs Force-push (queued) / Send vs Queue (draft). */
  status: Status;
  /** Agent-advertised `/` commands, threaded into the row's inline editor. */
  commands: () => AvailableCommand[];
}): React.JSX.Element {
  // Default expanded. Collapsed state is local + ephemeral (resets on session
  // switch / remount) — the count stays visible either way.
  const [collapsed, setCollapsed] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const count = items.length;
  // Bridge the locally-edited QUEUED message id to the store so the auto-drain
  // holds that message (and everything behind it) until the edit finishes.
  // Drafts don't drain, so they need no hold. Clears on unmount / session switch.
  useEffect(() => {
    if (kind !== "queued") return undefined;
    setQueueEditing(sessionId, editingId);
    return (): void => setQueueEditing(sessionId, null);
  }, [kind, sessionId, editingId]);
  // Drag-to-reorder. For the QUEUE, a drag behaves like an edit: hold the head so
  // the WHOLE queue pauses (drain is front-to-back) until the drop commits the
  // new order and releases. Drafts don't drain, so no hold.
  const byId = new Map(items.map((m) => [m.id, m]));
  const sortable = useSortable({
    ids: items.map((m) => m.id),
    onReorder: (order) =>
      kind === "queued" ? reorderQueue(sessionId, order) : reorderDrafts(sessionId, order),
    onDragStart:
      kind === "queued"
        ? (): void => {
            const head = items[0];
            if (head) setQueueEditing(sessionId, head.id);
          }
        : undefined,
    onDragEnd: kind === "queued" ? (): void => setQueueEditing(sessionId, null) : undefined,
  });
  const noun = kind === "queued" ? "Queued Message" : "Draft";
  return (
    <Box
      sx={{
        mb: 1,
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        // Drafts read as a quieter, dashed-feeling staging area vs the live queue.
        bgcolor: kind === "draft" ? "action.selected" : "action.hover",
        overflow: "hidden",
      }}
    >
      <Stack direction="row" alignItems="center" sx={{ pl: 0.5, pr: 0.75, py: 0.25 }}>
        <IconButton
          size="small"
          aria-label={collapsed ? "expand" : "collapse"}
          onClick={(): void => setCollapsed((c) => !c)}
          sx={{ flexShrink: 0 }}
        >
          {collapsed ? <ChevronRight fontSize="small" /> : <ExpandMore fontSize="small" />}
        </IconButton>
        <Typography
          variant="caption"
          sx={{ fontWeight: 600, flex: 1, minWidth: 0, cursor: "pointer" }}
          onClick={(): void => setCollapsed((c) => !c)}
        >
          {count} {noun}{count === 1 ? "" : "s"}
        </Typography>
        {kind === "draft" && (
          <Button
            size="small"
            color="primary"
            onClick={(): void => activateAllDrafts(sessionId)}
            sx={{ textTransform: "none", minWidth: 0, px: 0.75 }}
          >
            Send all
          </Button>
        )}
        <Button
          size="small"
          color="inherit"
          onClick={(): void =>
            kind === "queued" ? clearQueue(sessionId) : clearDrafts(sessionId)}
          sx={{ textTransform: "none", color: "text.secondary", minWidth: 0, px: 0.75 }}
        >
          Clear All
        </Button>
      </Stack>
      <Collapse in={!collapsed}>
        <Stack
          spacing={0.5}
          sx={{
            px: 0.5,
            pb: 0.5,
            // Cap so a long backlog scrolls instead of pushing the editor off
            // a phone viewport; the editor must always stay reachable.
            maxHeight: "30vh",
            overflowY: "auto",
          }}
        >
          {sortable.order.map((id) => {
            const m = byId.get(id);
            if (!m) return null;
            return (
              <Stack
                key={m.id}
                ref={sortable.registerItem(m.id)}
                style={sortable.itemStyle(m.id)}
                direction="row"
                alignItems="center"
                spacing={0.5}
              >
                {/* Leading grip — drag to reorder; hidden while this row is being
                    edited (the edit field owns the row then). */}
                {editingId !== m.id && (
                  <Box
                    {...sortable.handleProps(m.id)}
                    aria-label="Drag to reorder"
                    sx={{ display: "flex", alignItems: "center", color: "text.disabled", flexShrink: 0 }}
                  >
                    <DragIndicator fontSize="small" />
                  </Box>
                )}
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <PendingRow
                    kind={kind}
                    sessionId={sessionId}
                    message={m}
                    status={status}
                    commands={commands}
                    editing={editingId === m.id}
                    onEdit={(): void => setEditingId(m.id)}
                    onEditDone={(): void => setEditingId(null)}
                  />
                </Box>
              </Stack>
            );
          })}
        </Stack>
      </Collapse>
    </Box>
  );
}

// One queued prompt. Read mode shows the (clamped) text + a primary action +
// Edit / Delete. Edit mode swaps in a small multiline field (Enter saves, Esc
// cancels). The primary action depends on whether the session can take a turn
// right now: dispatchable → a plain "Send now" (sends immediately, revives a
// dead session); busy → a warning-coloured "Force push" that interrupts the
// running turn and runs this prompt next — gated behind a confirm popover
// because cancelling discards the in-flight turn's progress.
function PendingRow({
  kind,
  sessionId,
  message,
  status,
  commands,
  editing,
  onEdit,
  onEditDone,
}: {
  kind: "queued" | "draft";
  sessionId: string;
  message: QueuedMessage;
  status: Status;
  commands: () => AvailableCommand[];
  editing: boolean;
  onEdit: () => void;
  onEditDone: () => void;
}): React.JSX.Element {
  const [draft, setDraft] = useState(message.text);
  // Local attachments while editing, seeded from the queued message. The edit
  // box is the SAME ComposerEditor as the main composer, so a queued prompt can
  // gain/lose images here too (pasted screenshots, picked files).
  const [editAttachments, setEditAttachments] = useState<Attachment[]>(
    message.attachments,
  );
  const editorRef = useRef<ComposerEditorHandle>(null);
  // Confirm popover for force push (anchored to the Bolt button). Null = closed.
  const [confirmAnchor, setConfirmAnchor] = useState<HTMLElement | null>(null);
  // "running" is the idle-ready state; "exited"/"crashed" dispatch a revive.
  // Anything else ("busy"/"starting") has an in-flight turn → force push.
  const dispatchable = status === "running" || status === "exited" || status === "crashed";
  // Touch → native textarea (correct IME); desktop → CodeMirror.
  const touchInput = useTouchComposer();
  // Focus the editor when the row enters edit mode. useLayoutEffect, NOT
  // useEffect: a passive effect runs after paint, outside the tap's user-
  // activation window, so iOS Safari silently refuses to raise the keyboard for
  // the programmatic focus — the edit box opened but stayed unfocused. A layout
  // effect runs synchronously in the same task as the Edit-button click, before
  // paint; @uiw creates the CM view in its own (child) layout effect, which
  // fires first, so the view already exists here. Focusing from here keeps it
  // inside the gesture and pops the keyboard. (@uiw's own autoFocus wouldn't
  // help: it focuses from a passive useEffect — the same late timing.)
  useLayoutEffect(() => {
    // focusEnd, not focus: opening an existing draft/queued message should put
    // the caret at the end of its text so you continue typing, not at the start.
    if (editing) editorRef.current?.focusEnd();
  }, [editing]);
  if (editing) {
    const save = (): void => {
      if (kind === "draft") editDraft(sessionId, message.id, draft, editAttachments);
      else editQueued(sessionId, message.id, draft, editAttachments);
      onEditDone();
    };
    const cancel = (): void => {
      setDraft(message.text);
      setEditAttachments(message.attachments);
      onEditDone();
    };
    const addEditFiles = (files: File[]): void => {
      if (files.length === 0) return;
      void filesToAttachments(files).then((added) => {
        if (added.length > 0) setEditAttachments((prev) => [...prev, ...added]);
      });
    };
    return (
      <Paper variant="outlined" sx={{ p: 0.75 }}>
        {editAttachments.length > 0 && (
          <AttachmentPreviews
            attachments={editAttachments}
            onRemove={(id): void =>
              setEditAttachments((prev) => prev.filter((a) => a.id !== id))}
          />
        )}
        {touchInput ? (
          <ComposerTextarea
            ref={editorRef}
            value={draft}
            onChange={setDraft}
            onSubmit={save}
            sessionId={sessionId}
            commands={commands}
            placeholder="Edit queued message…"
            onPasteFiles={addEditFiles}
            onEscape={(): boolean => {
              cancel();
              return true;
            }}
          />
        ) : (
          <ComposerEditor
            ref={editorRef}
            // The edit box mounts fresh each time the row enters edit mode (the
            // read-mode branch has no ComposerEditor), so this seeds the current
            // text. Uncontrolled thereafter — onChange feeds `draft`, never back
            // into `value` (mirrors the main composer's latch-avoidance).
            value={message.text}
            onChange={setDraft}
            onSubmit={save}
            sessionId={sessionId}
            commands={commands}
            placeholder="Edit queued message…"
            onPasteFiles={addEditFiles}
            onEscape={(): boolean => {
              cancel();
              return true;
            }}
          />
        )}
        <Stack direction="row" spacing={0.5} justifyContent="flex-end" sx={{ mt: 0.5 }}>
          <Button
            size="small"
            color="inherit"
            onClick={cancel}
            sx={{ textTransform: "none" }}
          >
            Cancel
          </Button>
          <Button size="small" variant="contained" onClick={save} sx={{ textTransform: "none" }}>
            Save
          </Button>
        </Stack>
      </Paper>
    );
  }
  return (
    <Paper variant="outlined" sx={{ p: 0.75, display: "flex", alignItems: "flex-start", gap: 0.5 }}>
      <Box sx={{ flex: 1, minWidth: 0 }}>
        {message.text && (
          <Typography
            variant="body2"
            sx={{
              // Two-line clamp: keep rows compact so several queued prompts fit;
              // the full text is editable via the pencil.
              display: "-webkit-box",
              WebkitLineClamp: 2,
              WebkitBoxOrient: "vertical",
              overflow: "hidden",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {message.text}
          </Typography>
        )}
        {message.attachments.length > 0 && (
          <QueuedAttachmentChips attachments={message.attachments} />
        )}
      </Box>
      <Stack direction="row" sx={{ flexShrink: 0 }}>
        {kind === "draft" ? (
          // Activate: send now if the session's free, else queue it.
          <Tooltip title={dispatchable ? "Send" : "Add to queue"}>
            <IconButton
              size="small"
              color="primary"
              aria-label="send draft"
              onClick={(): void => activateDraft(sessionId, message.id)}
            >
              <Send fontSize="small" />
            </IconButton>
          </Tooltip>
        ) : (
          <>
            {dispatchable ? (
              <Tooltip title="Send now">
                <IconButton
                  size="small"
                  color="primary"
                  aria-label="send now"
                  onClick={(): void => requestSendQueued(sessionId, message.id)}
                >
                  <Send fontSize="small" />
                </IconButton>
              </Tooltip>
            ) : (
              <Tooltip title="Force push (interrupt & send)">
                <IconButton
                  size="small"
                  color="warning"
                  aria-label="force push"
                  onClick={(e): void => setConfirmAnchor(e.currentTarget)}
                >
                  <Bolt fontSize="small" />
                </IconButton>
              </Tooltip>
            )}
            <Popover
              open={confirmAnchor !== null}
              anchorEl={confirmAnchor}
              onClose={(): void => setConfirmAnchor(null)}
              anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
              transformOrigin={{ vertical: "top", horizontal: "right" }}
            >
              <Box sx={{ p: 1.5, maxWidth: 240 }}>
                <Typography variant="body2" sx={{ mb: 1 }}>
                  Stop the current turn and send this message now? The agent's
                  in-progress work is discarded.
                </Typography>
                <Stack direction="row" spacing={1} justifyContent="flex-end">
                  <Button
                    size="small"
                    color="inherit"
                    onClick={(): void => setConfirmAnchor(null)}
                    sx={{ textTransform: "none" }}
                  >
                    Cancel
                  </Button>
                  <Button
                    size="small"
                    variant="contained"
                    color="warning"
                    startIcon={<Bolt />}
                    onClick={(): void => {
                      forcePushQueued(sessionId, message.id);
                      setConfirmAnchor(null);
                    }}
                    sx={{ textTransform: "none" }}
                  >
                    Force push
                  </Button>
                </Stack>
              </Box>
            </Popover>
            <Tooltip title="Move to drafts">
              <IconButton
                size="small"
                aria-label="move to drafts"
                onClick={(): void => queuedToDraft(sessionId, message.id)}
              >
                <EditNoteOutlined fontSize="small" />
              </IconButton>
            </Tooltip>
          </>
        )}
        <Tooltip title="Edit">
          <IconButton size="small" aria-label="edit message" onClick={onEdit}>
            <EditOutlined fontSize="small" />
          </IconButton>
        </Tooltip>
        <Tooltip title="Remove">
          <IconButton
            size="small"
            aria-label="remove message"
            onClick={(): void =>
              kind === "draft"
                ? removeDraft(sessionId, message.id)
                : removeQueued(sessionId, message.id)}
          >
            <Close fontSize="small" />
          </IconButton>
        </Tooltip>
      </Stack>
    </Paper>
  );
}

function ConfigChipSkeletons(): React.JSX.Element {
  // Three skeletons sized to the typical chip widths (Bypass Permissions ≈
  // 160px, Default (recommended) ≈ 170px, High ≈ 80px). Keeps the row's
  // visual rhythm stable when the real chips replace them.
  const widths = [148, 168, 76];
  return (
    <>
      {widths.map((w, i) => (
        <Skeleton
          key={i}
          variant="rounded"
          width={w}
          height={36}
          animation="wave"
          sx={{ flexShrink: 0, borderRadius: 1 }}
        />
      ))}
    </>
  );
}

function ConfigOptionChip({
  option,
  disabled,
  onSelect,
}: {
  option: ConfigOption;
  disabled: boolean;
  onSelect: (value: string | boolean) => void;
}): React.JSX.Element {
  // Desktop-only: ComposerSheet handles touch viewports now, so this just
  // needs the anchored Menu it always had.
  const [anchor, setAnchor] = useState<null | HTMLElement>(null);
  const current = useMemo(
    () =>
      option.options.find((o) => o.value === option.currentValue) ??
      option.options[0],
    [option.options, option.currentValue],
  );
  return (
    <>
      <Button
        size="small"
        variant="outlined"
        color="inherit"
        disabled={disabled}
        endIcon={<ExpandMore fontSize="medium" />}
        onClick={(e): void => setAnchor(e.currentTarget)}
        sx={{
          textTransform: "none",
          ...TOOLBAR_MIN_H,
          px: 1.25,
          flexShrink: 0,
          fontWeight: 500,
          borderColor: "divider",
        }}
      >
        {current?.name ?? String(option.currentValue)}
      </Button>
      <Menu
        anchorEl={anchor}
        open={!!anchor}
        onClose={(): void => setAnchor(null)}
        slotProps={{ paper: { sx: { maxWidth: 360 } } }}
      >
        {option.options.map((o) => (
          <MenuItem
            key={String(o.value)}
            selected={o.value === option.currentValue}
            onClick={(): void => {
              onSelect(o.value);
              setAnchor(null);
            }}
            sx={{ alignItems: "flex-start", whiteSpace: "normal" }}
          >
            <Box>
              <Typography variant="body2" sx={{ fontWeight: 500 }}>
                {o.name}
              </Typography>
              {o.description && (
                <Typography variant="caption" color="text.secondary">
                  {o.description}
                </Typography>
              )}
            </Box>
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

// Unified bottom sheet for touch viewports: every action lives here, none
// in a visible inline row. ChatGPT / DeepSeek / Gemini all collapse their
// composer controls behind a single `+` because chip rows wrap awkwardly
// on iPad portrait (820px) and break entirely on a 390px iPhone, while
// the bottom-sheet pattern is iOS-native muscle memory.
function ComposerSheet({
  open,
  onClose,
  session,
  options,
  loading,
  dead,
  onSelectOption,
}: {
  open: boolean;
  onClose: () => void;
  session: SessionMeta | undefined;
  options: ConfigOption[];
  loading: boolean;
  dead: boolean;
  onSelectOption: (configId: string, value: string | boolean) => void;
}): React.JSX.Element {
  const navbarAtBottom = useNavbarAtBottom();
  return (
    <BottomSheet open={open} onClose={onClose} forceSheet={navbarAtBottom}>
      {session && <SessionInfoSection session={session} />}
      {(loading || options.length > 0) && (
        <>
          <Divider />
          <Box sx={{ py: 1.5 }}>
            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
            >
              Agent
            </Typography>
            {loading ? (
              <Stack
                direction="row"
                spacing={1.5}
                alignItems="center"
                sx={{ py: 1, color: "text.secondary" }}
              >
                <CircularProgress size={16} />
                <Typography variant="body2">Loading agent options…</Typography>
              </Stack>
            ) : (
              // Selecting a value does NOT close the sheet — mode, model, and
              // effort are commonly changed together, and each <Select> already
              // closes its own menu on pick. The user dismisses the sheet by
              // tapping outside once they're done.
              <Stack spacing={2} sx={{ mt: 1.5 }}>
                {options.map((opt) => (
                  <ConfigSheetDropdown
                    key={opt.id}
                    option={opt}
                    disabled={dead}
                    onSelect={(value): void => onSelectOption(opt.id, value)}
                  />
                ))}
              </Stack>
            )}
          </Box>
        </>
      )}
    </BottomSheet>
  );
}

// Read-only session metadata at the top of the options sheet. This used to
// live behind a long-press on the mobile title bar (a gesture nobody found),
// so it now rides the one popup the user already opens to change mode / model
// / effort. Desktop shows the same facts in the persistent sidebar, so this
// section only renders inside the compact-tier sheet.
function SessionInfoSection({
  session,
}: {
  session: SessionMeta;
}): React.JSX.Element {
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: "Provider", value: session.provider },
    { label: "Working dir", value: session.cwd, mono: true },
    { label: "Origin", value: originLabel(session.origin) },
    { label: "Status", value: session.status },
    { label: "Session id", value: session.id, mono: true },
  ];
  return (
    <>
      <Box sx={{ pt: 0.5, pb: 0.25 }}>
        <Typography
          variant="overline"
          color="text.secondary"
          sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
        >
          Session
        </Typography>
      </Box>
      <List dense disablePadding>
        {rows.map((r) => (
          <SheetDetailRow
            key={r.label}
            label={r.label}
            value={r.value}
            mono={r.mono === true}
          />
        ))}
      </List>
    </>
  );
}

function SheetDetailRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string;
  mono?: boolean;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        py: 0.75,
        display: "flex",
        gap: 2,
        alignItems: "baseline",
      }}
    >
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ minWidth: 96, flexShrink: 0 }}
      >
        {label}
      </Typography>
      <Typography
        variant="body2"
        sx={{
          flex: 1,
          minWidth: 0,
          wordBreak: "break-word",
          fontFamily: mono ? MONO : "inherit",
        }}
      >
        {value}
      </Typography>
    </Box>
  );
}

// One labelled dropdown per agent option (mode / model / effort). Collapses
// what used to be an always-expanded radio list — the sheet stays short even
// when an agent advertises a dozen models. ACP option values may be string OR
// boolean, so the <Select> is keyed on String(value) and mapped back to the
// original type on change.
function ConfigSheetDropdown({
  option,
  disabled,
  onSelect,
}: {
  option: ConfigOption;
  disabled: boolean;
  onSelect: (value: string | boolean) => void;
}): React.JSX.Element {
  const currentKey = String(option.currentValue);
  return (
    <TextField
      select
      fullWidth
      size="small"
      disabled={disabled}
      label={option.name}
      value={currentKey}
      onChange={(e): void => {
        const picked = option.options.find(
          (o) => String(o.value) === e.target.value,
        );
        if (picked) onSelect(picked.value);
      }}
      {...(option.description ? { helperText: option.description } : {})}
    >
      {option.options.map((o) => (
        <MenuItem key={String(o.value)} value={String(o.value)}>
          {o.name}
        </MenuItem>
      ))}
    </TextField>
  );
}

const MONO = "ui-monospace, SFMono-Regular, Menlo, monospace";

// Find the most recent `available_commands_update` payload in the session's
// event log. Walks in reverse so the cost is at most one event when the
// agent already advertised, and the empty-array baseline is cheap on first
// connect.
function latestAvailableCommands(timeline: Envelope[]): AvailableCommand[] {
  for (let i = timeline.length - 1; i >= 0; i -= 1) {
    const env = timeline[i];
    if (env && env.kind === "update") {
      const u = env.update as AcpUpdate;
      if (u.sessionUpdate === "available_commands_update" && Array.isArray(u.availableCommands)) {
        return u.availableCommands;
      }
    }
  }
  return [];
}

