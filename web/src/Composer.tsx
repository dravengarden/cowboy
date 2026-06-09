import {
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  alpha,
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
  keyframes,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Paper,
  Popover,
  Skeleton,
  Snackbar,
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
  CloseFullscreen,
  DragIndicator,
  DriveFileMoveOutlined,
  EditNoteOutlined,
  EditOutlined,
  ExpandMore,
  InsertDriveFileOutlined,
  MoreVert,
  OpenInFull,
  Refresh,
  Send,
  Stop,
  SwapVert,
  Tune,
  Undo,
  VerticalAlignBottom,
} from "@mui/icons-material";
import { ComposerEditor, type ComposerEditorHandle } from "./ComposerEditor";
import { ComposerTextarea, useTouchComposer } from "./ComposerTextarea";
import { Kbd, useConfirmEnter } from "./Kbd";
import { DRAFT_LABEL, ENTER_LABEL, MOD_LABEL } from "./platform";
import { openLightbox } from "./ResourceLightbox";
import { PlanDock } from "./PlanDock";
import { TurnStatusOverlay } from "./TurnStatusOverlay";
import { latestPlan } from "./derive";
import { DetentSheet } from "./_shell";
import { toggleComposerExpanded, useComposerExpanded } from "./composerExpand";
import { setVimMode } from "./vimModeStore";
import { requestStickToBottom, setSticky, useSticky } from "./stickyStore";
import { claimKeyboard } from "./keyboardClaim";
import { useVimSetting } from "./vimSetting";
import { type Attachment, filesToAttachments } from "./attachments";
import {
  activateAllDrafts,
  activateDraft,
  addDraft,
  clearDrafts,
  clearQueue,
  discardQueued,
  editDraft,
  editQueued,
  forcePrompt,
  forcePushQueued,
  moveDraft,
  type QueuedMessage,
  queuedToDraft,
  removeDraft,
  removeQueued,
  reorderDrafts,
  reorderQueue,
  requestSendQueued,
  retryQueued,
  send,
  setQueueEditing,
  submitPrompt,
  useInferenceConfig,
  useStore,
} from "./store";
import { haptic } from "./haptic";
import { useSortable } from "./useSortable";
import { getDraft, setDraft } from "./draftStore";
import { useNavbarAtBottom } from "./navbarSettings";
import { useReadingSettings } from "./readingSettings";
import { originLabel } from "./protocol";
import type {
  AcpUpdate,
  AvailableCommand,
  ConfigOption,
  Envelope,
  SessionMeta,
  Status,
} from "./protocol";
import { Sheet } from "./Sheet";
import {
  persisted,
  type Store,
  useStore as usePrefStore,
} from "./_store/mod.ts";

// Per-panel-kind collapse pref (app-level, per-device, never synced). One
// persisted store per key ("cowboy:<kind>-collapsed"), "1"/"0" legacy format
// preserved. Memoized so each kind has a single shared store instance.
const collapseStores = new Map<string, Store<boolean>>();
function collapseStore(key: string): Store<boolean> {
  let s = collapseStores.get(key);
  if (s === undefined) {
    s = persisted(key, false, {
      serialize: (v) => (v ? "1" : "0"),
      deserialize: (raw) => raw === "1",
    });
    collapseStores.set(key, s);
  }
  return s;
}

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

// Toolbar icon buttons take the unified 44px box + fixed 24px glyph from the
// global MuiIconButton theme override (same as the session-list buttons); this
// only keeps them from shrinking in the flex toolbar row.
const TOOLBAR_ICON_BTN = { flexShrink: 0 } as const;
const TOOLBAR_MIN_H = {
  minHeight: 34,
  "@media (pointer: coarse)": { minHeight: 40 },
} as const;

// The mobile fullscreen compose/edit docked BAR — ONE shared bar so Compose and
// Edit look + behave identically (Send, not Cancel/Save). Stacked in the sticky
// docked footer (above the keyboard): attachment thumbnails, then — only when the
// ⚙ toggle is open — the config dropdowns (folded by default so the bar stays a
// single compact row), then the action row: slash / @ / attach / ⚙ on the left,
// Send (submits, or saves the edit) on the right. width:100% + the breakout
// divider make it a full-width docked bar (the shared sheet footer is
// justify-end, which otherwise clusters it).
function ComposeBar(
  {
    dead,
    sendable,
    onTrigger,
    onSend,
    options = [],
    showSkeleton = false,
    onConfig,
    onAttach,
    attachments = [],
    onRemoveAttachment,
    onSaveDraft,
    onCollapse,
    onForcePush,
  }: {
    readonly dead: boolean;
    readonly sendable: boolean;
    readonly onTrigger: (trigger: string) => void;
    readonly onSend: () => void;
    readonly options?: ConfigOption[];
    readonly showSkeleton?: boolean;
    readonly onConfig?: ((configId: string, value: string | boolean) => void) | undefined;
    readonly onAttach?: (() => void) | undefined;
    readonly attachments?: Attachment[];
    readonly onRemoveAttachment?: ((id: string) => void) | undefined;
    readonly onSaveDraft?: (() => void) | undefined;
    readonly onCollapse?: (() => void) | undefined;
    /** Receives the ⋮ button so the caller can anchor its force-push confirm. */
    readonly onForcePush?: ((anchor: HTMLElement) => void) | undefined;
  },
): React.JSX.Element {
  // Config dropdowns fold behind the ⚙ toggle — the bar stays one compact row,
  // and tapping ⚙ reveals the dropdowns ABOVE the action row.
  const [configOpen, setConfigOpen] = useState(false);
  const hasConfig = showSkeleton || options.length > 0;
  return (
    <Stack
      direction="column"
      spacing={0.75}
      sx={{
        width: "100%",
        mx: -2,
        px: 2,
        pt: 1,
        borderTop: (t) => `1px solid ${t.palette.divider}`,
      }}
    >
      {/* Attachment thumbnails live in the sticky docked bar (above the action
          row), NOT in the scrolling editor body, so they stay put while you write. */}
      {attachments.length > 0 && onRemoveAttachment && (
        <AttachmentPreviews attachments={attachments} onRemove={onRemoveAttachment} />
      )}
      {hasConfig && configOpen && (
        <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5 }}>
          {showSkeleton ? <ConfigChipSkeletons /> : (
            options.map((opt) => (
              <ConfigOptionChip
                key={opt.id}
                option={opt}
                disabled={dead}
                onSelect={(value): void => onConfig?.(opt.id, value)}
              />
            ))
          )}
        </Box>
      )}
      <Stack direction="row" alignItems="center" spacing={0.5}>
        {/* Left icon group takes the available width and scrolls if it overflows,
            so Send (pinned right, never shrinks) is ALWAYS visible — the icons are
            flexShrink:0, so otherwise the busy-state force-push pushed Send off. */}
        <Box
          sx={{
            flex: 1,
            minWidth: 0,
            display: "flex",
            alignItems: "center",
            gap: 0.5,
            overflowX: "auto",
            scrollbarWidth: "none",
            "&::-webkit-scrollbar": { display: "none" },
          }}
        >
          <Tooltip title="Slash command / skill">
            <span>
              <IconButton
                aria-label="slash command"
                disabled={dead}
                sx={TOOLBAR_ICON_BTN}
                onClick={(): void => onTrigger("/")}
              >
                <Box
                  component="span"
                  sx={{ fontSize: "1.375rem", fontWeight: 700, lineHeight: 1 }}
                >
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
                onClick={(): void => onTrigger("@")}
              >
                <AlternateEmail />
              </IconButton>
            </span>
          </Tooltip>
          {onAttach && (
            <Tooltip title="Attach image or file">
              <span>
                <IconButton
                  aria-label="attach image or file"
                  disabled={dead}
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onAttach}
                >
                  <AttachFile />
                </IconButton>
              </span>
            </Tooltip>
          )}
          {hasConfig && (
            <Tooltip title="Options">
              <span>
                <IconButton
                  aria-label="options"
                  disabled={dead}
                  color={configOpen ? "primary" : "default"}
                  sx={TOOLBAR_ICON_BTN}
                  onClick={(): void => setConfigOpen((o) => !o)}
                >
                  <Tune />
                </IconButton>
              </span>
            </Tooltip>
          )}
          {/* Secondary actions are direct buttons on the bar (not folded behind a
              ⋮ — the user wanted them visible). Force push only while busy/starting. */}
          {onForcePush && (
            <Tooltip title="Force push">
              <span>
                <IconButton
                  color="primary"
                  aria-label="force push"
                  sx={TOOLBAR_ICON_BTN}
                  onClick={(e): void => onForcePush(e.currentTarget)}
                >
                  <Bolt />
                </IconButton>
              </span>
            </Tooltip>
          )}
          {onSaveDraft && (
            <Tooltip title="Save as draft">
              <span>
                <IconButton
                  aria-label="save as draft"
                  disabled={!sendable}
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onSaveDraft}
                >
                  <EditNoteOutlined />
                </IconButton>
              </span>
            </Tooltip>
          )}
          {onCollapse && (
            <Tooltip title="Exit fullscreen">
              <span>
                <IconButton
                  aria-label="exit fullscreen"
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onCollapse}
                >
                  <CloseFullscreen />
                </IconButton>
              </span>
            </Tooltip>
          )}
        </Box>
        <Tooltip title="Send">
          <span>
            <IconButton
              color="primary"
              aria-label="send"
              disabled={!sendable}
              sx={TOOLBAR_ICON_BTN}
              onClick={onSend}
            >
              <Send />
            </IconButton>
          </span>
        </Tooltip>
      </Stack>
    </Stack>
  );
}

export function Composer({
  sessionId,
  status,
  onOpenInfo,
}: {
  sessionId: string;
  status: Status;
  onOpenInfo: () => void;
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
  // `queues`/`drafts` already merge the server rows with this device's optimistic
  // (sending/failed) rows via the queue sync client (commitQueue) — server rows
  // first, optimistic rebased after, reconciled out the instant their cmid lands.
  const queue = queues.get(sessionId) ?? [];
  const draftList = drafts.get(sessionId) ?? [];
  // The agent's current plan, pinned above the queue as a collapsible dock so
  // task progress stays in view without scrolling the transcript. null = no plan.
  const plan = useMemo(() => latestPlan(timelines.get(sessionId) ?? []), [
    timelines,
    sessionId,
  ]);
  // Manual dismiss: keyed on the plan's step list so it stays gone as the agent
  // updates statuses, but a genuinely new plan (different steps) reappears.
  const [dismissedPlanKey, setDismissedPlanKey] = useState<string | null>(null);
  // Show the plan unless (a) the user dismissed this exact plan, or (b) it's
  // fully complete AND the user has already moved on to a new turn — ACP never
  // signals "plan done", so a finished plan would otherwise linger forever.
  const showPlan = plan !== null &&
    plan.key !== dismissedPlanKey &&
    !(plan.supersededByUserTurn &&
      plan.entries.every((e) => e.status === "completed"));
  // The active session's metadata, surfaced read-only inside the options
  // sheet (mobile's "session settings" popup). Desktop shows the same facts
  // in the always-visible sidebar, so the sheet — and this lookup — only
  // matters on the compact tier.
  const session = sessions.find((s) => s.id === sessionId);
  const inferenceConfig = useInferenceConfig();
  const hasJudgeKey = inferenceConfig.some((c) =>
    c.provider === "deepseek" && c.key_set
  );
  const theme = useTheme();
  // Touch tier collapses the agent config into a single Tune button — tapping
  // it opens a BottomSheet with the session info + every config option in one
  // place. Inspired by ChatGPT / DeepSeek / Gemini: chips wrap awkwardly on
  // iPad portrait (820px) and are completely unreadable on a 390px iPhone, so
  // the sheet pattern wins on every sub-desktop viewport. Desktop keeps the
  // inline chip row — there's room.
  const compact = useMediaQuery(theme.breakpoints.down("lg"));
  const [sheetOpen, setSheetOpen] = useState(false);
  // Mobile-only fullscreen compose: the ↗ opens a near-full-screen sheet (the
  // first-class long-form / future-markdown editor). Desktop keeps the Zed-style
  // inline expand instead (composeFs is never set true there).
  const [composeFs, setComposeFs] = useState(false);
  // Focus the fullscreen editor when it opens (the inline one just unmounted, so
  // the shared editorRef now points here). Small delay so the sheet has mounted;
  // on touch the keyboard may need one tap if it lands outside the gesture window.
  useEffect(() => {
    if (!composeFs) return undefined;
    const t = globalThis.setTimeout(() => editorRef.current?.focusEnd(), 60);
    return () => globalThis.clearTimeout(t);
  }, [composeFs]);
  // Stopping a running turn is confirmed through a modal (Enter confirms, Esc
  // dismisses) — clicking Stop or pressing Esc in the editor opens it, rather
  // than cancelling on a single stray click/keypress.
  const [cancelOpen, setCancelOpen] = useState(false);
  // The composer's overlaid ⋮ kebab (Save draft / Force push) anchor. The
  // secondary actions live here so the input's bottom-right shows only the
  // primary Send/Queue + (busy) Stop, matching the pending-row layout.
  const [actionsMenu, setActionsMenu] = useState<HTMLElement | null>(null);
  // Long-press-send → force-push: hold the Queue button ~450ms to pop a confirm
  // that interrupts the running turn and runs this prompt next (skipping the
  // queue). `holding` drives the fill ring; `forceAnchor` anchors the popover.
  const [holding, setHolding] = useState(false);
  const [forceAnchor, setForceAnchor] = useState<HTMLElement | null>(null);
  // The Queue button — also the anchor for a KEYBOARD-triggered force-push (held
  // ⌘⏎), so the confirm rises from the same spot whether opened by hold or key.
  const queueBtnRef = useRef<HTMLButtonElement | null>(null);
  const lpTimer = useRef<number | undefined>(undefined);
  // Set when the hold crosses the threshold, so the trailing click (pointerup
  // fires onClick) is suppressed instead of also queuing the message.
  const lpFired = useRef(false);
  const lpStart = useRef<{ x: number; y: number } | null>(null);
  // "Move draft to another session" (the parked-in-the-wrong-session fix). Owned
  // HERE, not in the drafts panel, so the undo snackbar survives the panel
  // unmounting when the LAST draft leaves this session. `moveSrcId` = the draft
  // whose destination picker is open; `moveUndo` backs the post-move snackbar so
  // a mis-tapped destination is one tap to reverse.
  const [moveSrcId, setMoveSrcId] = useState<string | null>(null);
  const [moveUndo, setMoveUndo] = useState<
    { id: string; toId: string; toTitle: string } | null
  >(null);
  const otherSessions = sessions.filter((s) => s.id !== sessionId);
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
  // Interrupted is a dead/resumable state too (a turn cut off by a daemon
  // restart) — the composer treats it like exited/crashed: "send to resume".
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
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
  const showSkeleton = !dead && options.length === 0 &&
    (starting || status === "running");

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
  // Expand toggle (desktop only — gated where rendered). Persisted per device.
  const expanded = useComposerExpanded();
  // Auto-scroll / stick-to-bottom state for the toolbar's scroll-to-bottom toggle.
  const sticky = useSticky(sessionId);
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

  // --- Long-press → force-push ------------------------------------------------
  const LP_MS = 450;
  function clearLongPress(): void {
    if (lpTimer.current !== undefined) {
      globalThis.clearTimeout(lpTimer.current);
      lpTimer.current = undefined;
    }
    lpStart.current = null;
    setHolding(false);
  }
  function onForcePointerDown(e: ReactPointerEvent<HTMLButtonElement>): void {
    if (!sendable) return;
    const el = e.currentTarget;
    lpFired.current = false;
    lpStart.current = { x: e.clientX, y: e.clientY };
    setHolding(true);
    lpTimer.current = globalThis.setTimeout(() => {
      lpTimer.current = undefined;
      lpFired.current = true;
      setHolding(false);
      haptic();
      setForceAnchor(el);
    }, LP_MS);
  }
  function onForcePointerMove(e: ReactPointerEvent<HTMLButtonElement>): void {
    const s = lpStart.current;
    // A finger that drifts is a scroll/drag, not a press — cancel the hold.
    if (s !== null && Math.hypot(e.clientX - s.x, e.clientY - s.y) > 10) {
      clearLongPress();
    }
  }
  function onQueueClick(): void {
    // A completed long-press already opened the confirm; swallow the trailing
    // click so it doesn't ALSO queue the message.
    if (lpFired.current) {
      lpFired.current = false;
      return;
    }
    submit();
  }
  function confirmForce(): void {
    setForceAnchor(null);
    if (!sendable) return;
    haptic();
    forcePrompt(sessionId, text.trimEnd(), attachments);
    editorRef.current?.clear();
    setText("");
    setAttachments([]);
  }
  // Enter confirms the force-push popover (it doesn't autofocus a button the way
  // the Dialogs do). Held-⌘⏎ repeats are ignored inside the hook, so the still-
  // down Enter that opened it can't self-confirm — a fresh press does.
  useConfirmEnter(forceAnchor !== null, confirmForce);
  useEffect(() => (): void => {
    if (lpTimer.current !== undefined) globalThis.clearTimeout(lpTimer.current);
  }, []);

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
        // Bottom: reserve the FULL home-indicator inset so the action row's
        // far-edge buttons (slash / send) sit ABOVE the indicator and stay easy to
        // tap — an earlier `inset − 20px` let them reach into the indicator /
        // rounded-corner zone, which read as "hard to tap" on iPad. Floored to 10px
        // off-device.
        // Match the inter-panel gap (PendingPanel mb: 1 = 8px): the space above
        // the first pending panel reads as the same "panel spacing" as between
        // the queue + drafts panels, instead of a tighter xs top.
        pt: 1,
        // Bottom inset only when the composer is the bottom-most element. With
        // the navbar at the bottom it sits below us and owns the home-indicator
        // inset, so we drop to a plain (tight) gap.
        pb: navbarAtBottom
          ? 0
          : { xs: "max(env(safe-area-inset-bottom), 10px)", sm: 1.5 },
        pl: `max(env(safe-area-inset-left), ${padding}px)`,
        pr: `max(env(safe-area-inset-right), ${padding}px)`,
        borderColor: "divider",
        // TRANSPARENT in BOTH modes. The composer floats over ONE frosted slab
        // rendered behind it in App.tsx (the bottom slab); it therefore adds no own
        // tint / backdrop-filter / up-shadow / top border — doing so would double the
        // blur. The transcript is the full-height background and scrolls UNDER it.
        // (Top mode used to self-frost back when it sat in normal flow with nothing
        // scrolling under it; the overlay layout now puts the transcript behind it,
        // so the slab owns the glass — same as mobile.)
        bgcolor: "transparent",
        borderTop: 0,
        position: "relative", // anchor for Popper portal placement
      }}
    >
      {
        /* Agent plan (very top): a pinned, collapsible progress summary so the
          task's plan stays visible above the queue without scrolling. Hidden
          when there's no plan, when dismissed, or when a finished plan has been
          superseded by a new turn (see showPlan). */
      }
      {showPlan && plan && (
        <PlanDock
          entries={plan.entries}
          onDismiss={(): void => setDismissedPlanKey(plan.key)}
        />
      )}
      {
        /* Confirm-detect: the unified turn-status overlay (floats above the
          composer). It decides its own visibility — awaiting / done / interrupted
          / error / no-key, hidden while working — so it's rendered unconditionally. */
      }
      <TurnStatusOverlay
        sessionId={sessionId}
        status={status}
        awaitingUser={session?.awaiting_user ?? false}
        done={session?.done ?? false}
        judging={session?.judging ?? false}
        queue={queue}
        hasKey={hasJudgeKey}
        onFocusComposer={(): void => editorRef.current?.focus()}
        onConfigure={onOpenInfo}
      />
      {
        /* Queued prompts (top): while the agent is busy, messages stack here and
          drain one per turn-end. Hidden when empty. */
      }
      {queue.length > 0 && (
        <PendingPanel
          kind="queued"
          sessionId={sessionId}
          items={queue}
          status={status}
          commands={(): AvailableCommand[] => availableCommands}
        />
      )}
      {
        /* Drafts (below the queue, above the input): parked messages the user
          holds and activates on demand. Persisted across reloads. */
      }
      {draftList.length > 0 && (
        <PendingPanel
          kind="draft"
          sessionId={sessionId}
          items={draftList}
          status={status}
          commands={(): AvailableCommand[] => availableCommands}
          // Only offer "move" when there's somewhere to move to.
          onMoveDraft={otherSessions.length > 0
            ? (id: string): void => setMoveSrcId(id)
            : undefined}
        />
      )}
      {
        /* Staged attachments (image thumbnails / file chips) sit above the editor
          so they read as "what will be sent with this message". */
      }
      {attachments.length > 0 && (
        <AttachmentPreviews
          attachments={attachments}
          onRemove={removeAttachment}
        />
      )}
      {
        /* Hidden multi-file picker driven by the paperclip button. `accept` is
          left open so any file type can be attached (images embed inline, other
          files ride as ACP resource blocks — see attachments.ts). */
      }
      <input
        ref={fileInputRef}
        type="file"
        multiple
        hidden
        onChange={(e): void => {
          addFiles(Array.from(e.target.files ?? []));
          // Reset so picking the same file twice in a row still fires change.
          e.target.value = "";
          // NB: do NOT refocus the composer here — on iOS it leaves a phantom
          // keyboard-viewport shrink (dead gap below the UI). The picker drops the
          // keyboard regardless (platform limit); the user taps to resume.
        }}
      />
      {
        /* Input tier: native textarea on touch (CodeMirror's contenteditable
          strands IME pinyin on iOS — see ComposerTextarea), CodeMirror on
          desktop (vim + live @/​/ completion). Same ComposerEditorHandle ref. */
      }
      {/* The composer CARD (Zed-style): one outlined Paper owning the box — the
          editor sits borderless inside, the Send/Queue + Stop + ⋮ kebab overlay
          its bottom-right (`endInset` reserves text room). Transparent fill so it
          floats over the frosted bottom slab (a solid paper would hide the glass).
          A flex column so a later step can pin an inline toolbar to the bottom. */}
      <Paper
        variant="outlined"
        sx={{
          position: "relative",
          display: "flex",
          flexDirection: "column",
          bgcolor: "transparent",
        }}
      >
      {!composeFs && (touchInput
        ? (
          <ComposerTextarea
            ref={editorRef}
            // Controlled by `text` (a native textarea handles IME under control).
            value={text}
            onChange={setText}
            onSubmit={submit}
            onSaveDraft={saveDraft}
            sessionId={sessionId}
            commands={(): AvailableCommand[] => availableCommands}
            placeholder={dead
              ? "Send to resume this session…"
              : "Message the agent…"}
            onPasteFiles={addFiles}
            borderless
            endInset={36}
            onEscape={(): boolean => {
              if (busy) {
                setCancelOpen(true);
                return true;
              }
              return false;
            }}
          />
        )
        : (
          <ComposerEditor
            ref={editorRef}
            // Stable seed only (uncontrolled — see initialDraftText). NOT `text`.
            value={initialDraftText.current}
            onChange={setText}
            onSubmit={submit}
            onSaveDraft={saveDraft}
            borderless
            expanded={expanded}
            // Reserve a top-right gutter so no line runs under the ↗/↙ expand
            // toggle the card overlays at its top-right corner.
            endInset={36}
            // Hold ⌘⏎ while busy → the same force-push confirm the Queue button's
            // long-press opens, anchored to that button.
            holdToForce={busy || starting}
            onForceHold={(): void => {
              if (!sendable || queueBtnRef.current === null) return;
              haptic();
              setForceAnchor(queueBtnRef.current);
            }}
            sessionId={sessionId}
            commands={(): AvailableCommand[] => availableCommands}
            placeholder={dead
              ? "Send to resume this session…"
              : "Message the agent…"}
            vim={vim}
            onVimMode={setVimMode}
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
        ))}
        {/* Expand toggle, top-right INSIDE the card. DESKTOP: Zed-style inline
            expand — toggles a taller editor in place (flows through the
            --composer-h ResizeObserver). MOBILE: space is tight inline, so ↗ goes
            straight to the FULLSCREEN compose sheet (the first-class long-form /
            future-markdown editor). The editor reserves a right gutter (endInset)
            so text never runs under it. Glyph sized at the BUTTON level so it
            beats the global MuiIconButton `& .MuiSvgIcon-root: 1.5rem` override
            (a per-icon sx loses that specificity); rem so it tracks the font scale. */}
        <Tooltip
          title={touchInput
            ? "Fullscreen editor"
            : (expanded ? "Collapse editor" : "Expand editor")}
        >
          <IconButton
            size="small"
            aria-label={touchInput
              ? "fullscreen editor"
              : (expanded ? "collapse editor" : "expand editor")}
            onClick={touchInput
              ? (): void => {
                // Claim the keyboard IN-gesture (iOS) so it's already up when the
                // sheet's editor mounts + focuses; else the focus lands outside the
                // gesture window and the keyboard stays down (you'd have to tap).
                claimKeyboard();
                setComposeFs(true);
              }
              : toggleComposerExpanded}
            sx={{
              position: "absolute",
              top: 2,
              right: 2,
              zIndex: 1,
              color: "text.secondary",
              "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
            }}
          >
            {!touchInput && expanded ? <CloseFullscreen /> : <OpenInFull />}
          </IconButton>
        </Tooltip>
        {/* Inline bottom toolbar INSIDE the card (Zed layout): the / @ 📎 triggers
            + config on the left, a flex spacer, then the send/queue/stop + ⋮ action
            cluster pinned to the card's right edge. This replaces BOTH the old
            separate toolbar strip below the input AND the absolute send overlay —
            one cohesive card. `px`/`pb` (not the nav gutters) inset the row to the
            card's own edges. */}
        <Stack
          direction="row"
          alignItems="center"
          // Compact (mobile) is tight, so spread the icons space-evenly across the
          // whole row (edge padding included → nothing crammed at the right edge),
          // and @ folds out below to make room. Desktop keeps the tight left group
          // + config chips + the right-pinned action cluster.
          spacing={compact ? 0 : 0.5}
          sx={{
            px: 0.5,
            pb: 0.5,
            ...(compact && { justifyContent: "space-evenly" }),
            ...TOOLBAR_MIN_H,
          }}
        >
        {/* (Vim mode moved OUT of the toolbar into a Zed-style bottom status bar
            below — see the StatusBar at the card's bottom edge.) */}
        <Tooltip title="Slash command / skill">
          <span>
            <IconButton
              aria-label="slash command"
              disabled={dead}
              sx={TOOLBAR_ICON_BTN}
              onClick={(): void => editorRef.current?.insertTrigger("/")}
            >
              {
                /* rem, NOT px: the global font scale (useGlobalFontScale) grows
                  the app via the root font-size, so the sibling SvgIcons (rem)
                  scale but a px glyph wouldn't. 1.375rem = the old 22px at 100%,
                  tracking the ~1.5rem medium icons next to it. */
              }
              <Box
                component="span"
                sx={{ fontSize: "1.375rem", fontWeight: 700, lineHeight: 1 }}
              >
                /
              </Box>
            </IconButton>
          </span>
        </Tooltip>
        {/* @ folds out on compact (mobile) — the row is too tight, and typing
            "@" raises the same file picker. Desktop keeps the dedicated button. */}
        {!compact && (
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
        )}
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
        {compact
          ? (
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
          )
          : (
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
              {showSkeleton ? <ConfigChipSkeletons /> : (
                options.map((opt) => (
                  <ConfigOptionChip
                    key={opt.id}
                    option={opt}
                    disabled={dead}
                    onSelect={(value): void => {
                      send({
                        type: "set_config_option",
                        session_id: sessionId,
                        config_id: opt.id,
                        value,
                      });
                    }}
                  />
                ))
              )}
            </Stack>
          )}

        {/* Sticky / auto-scroll toggle — the persistent scroll-to-bottom control,
            rightmost of the left group. Default ON (active = primary, following the
            latest); inactive = muted. Tap while inactive → scroll to bottom + follow
            again; tap while active → stop following. The Transcript owns the actual
            scroll (stickyStore). Hover-only tooltip (disableFocus/Touch) so a tap on
            the most-tapped control doesn't pop the bubble. */}
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
              sticky ? setSticky(sessionId, false) : requestStickToBottom(sessionId)}
          >
            <VerticalAlignBottom />
          </IconButton>
        </Tooltip>

        {/* Spacer (desktop only) → pins the Stop + send/queue/⋮ cluster to the
            right edge while the left group stays left. On compact the row is
            space-evenly instead, so the spacer would defeat the even spread. */}
        {!compact && <Box sx={{ flex: 1 }} />}
        {/* Stop owns the turn while busy — sits just left of the send path. */}
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
        {/* Primary action: Send (idle) / Queue (busy — long-press → force push).
            Moved here from the old absolute overlay so the whole composer is one
            card; the long-press force-push ring + haptics are preserved. */}
        {busy || starting
          ? (
            <Box component="span" sx={{ position: "relative", display: "inline-flex" }}>
                <IconButton
                  ref={queueBtnRef}
                  color="primary"
                  aria-label="queue message"
                  disabled={!sendable}
                  sx={{
                    ...TOOLBAR_ICON_BTN,
                    transition: "transform .12s",
                    ...(holding && { transform: "scale(1.12)" }),
                  }}
                  onClick={onQueueClick}
                  onPointerDown={onForcePointerDown}
                  onPointerMove={onForcePointerMove}
                  onPointerUp={clearLongPress}
                  onPointerLeave={clearLongPress}
                  onPointerCancel={clearLongPress}
                >
                  <Send fontSize="small" />
                </IconButton>
                {holding && (
                  <Box
                    component="svg"
                    aria-hidden
                    viewBox="0 0 40 40"
                    sx={{
                      position: "absolute",
                      inset: 0,
                      width: "100%",
                      height: "100%",
                      pointerEvents: "none",
                      transform: "rotate(-90deg)",
                    }}
                  >
                    <Box
                      component="circle"
                      cx="20"
                      cy="20"
                      r="18"
                      fill="none"
                      strokeLinecap="round"
                      sx={{
                        stroke: "primary.main",
                        strokeWidth: 2.5,
                        strokeDasharray: 113,
                        strokeDashoffset: 113,
                        animation: "lpfill 450ms linear forwards",
                        "@keyframes lpfill": { to: { strokeDashoffset: 0 } },
                      }}
                    />
                  </Box>
                )}
              </Box>
          )
          : (
            <Tooltip title={`Send (${MOD_LABEL}${ENTER_LABEL})`}>
              <span>
                <IconButton
                  color="primary"
                  aria-label="send"
                  disabled={!sendable}
                  sx={TOOLBAR_ICON_BTN}
                  onClick={submit}
                >
                  <Send fontSize="small" />
                </IconButton>
              </span>
            </Tooltip>
          )}
        {/* ⋮ secondary actions (Save draft / keyboard force-push). Always rendered
            AND always clickable so the toolbar's icon COUNT never changes with input
            content — only the busy-state Stop is added/removed. Disable is per-item
            (inside the menu), not on the ⋮ itself, so the menu always opens. */}
        <IconButton
          aria-label="more actions"
          sx={TOOLBAR_ICON_BTN}
          onClick={(e): void => setActionsMenu(e.currentTarget)}
        >
          <MoreVert fontSize="small" />
        </IconButton>
        </Stack>
        {/* (Vim status moved to the app-wide bottom status bar — see App's
            StatusBar at the very bottom of the window, Zed/VSCode style.) */}
      </Paper>
      {
        /* The input overlay's ⋮ kebab — secondary actions that used to sit in the
          toolbar (Save draft) + the keyboard-only force-push, now discoverable. */
      }
      <Menu
        anchorEl={actionsMenu}
        open={actionsMenu !== null}
        onClose={(): void => setActionsMenu(null)}
        anchorOrigin={{ vertical: "top", horizontal: "right" }}
        transformOrigin={{ vertical: "bottom", horizontal: "right" }}
      >
        <MenuItem
          disabled={!sendable}
          onClick={(): void => {
            saveDraft();
            setActionsMenu(null);
          }}
        >
          <EditNoteOutlined fontSize="small" sx={{ mr: 1 }} />
          Save as draft
          <Kbd keys={`${DRAFT_LABEL}${ENTER_LABEL}`} />
        </MenuItem>
        {(busy || starting) && (
          <MenuItem
            onClick={(): void => {
              setActionsMenu(null);
              if (queueBtnRef.current) setForceAnchor(queueBtnRef.current);
            }}
          >
            <Bolt fontSize="small" sx={{ mr: 1 }} />
            Force push
          </MenuItem>
        )}
      </Menu>
      {
        /* Force-push confirm — opened by a completed long-press on Queue. Anchored
          to the button, rising above it. Confirm interrupts the running turn and
          runs this prompt next (skipping the queue). */
      }
      <Popover
        open={forceAnchor !== null}
        anchorEl={forceAnchor}
        onClose={(): void => setForceAnchor(null)}
        anchorOrigin={{ vertical: "top", horizontal: "center" }}
        transformOrigin={{ vertical: "bottom", horizontal: "center" }}
        slotProps={{
          paper: { sx: { mt: -1, maxWidth: 268, borderRadius: 2 } },
        }}
      >
        <Box sx={{ p: 1.5 }}>
          <Stack
            direction="row"
            spacing={0.75}
            alignItems="center"
            sx={{ mb: 0.5 }}
          >
            <Bolt fontSize="small" color="primary" />
            <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
              Force push
            </Typography>
          </Stack>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ display: "block", lineHeight: 1.5 }}
          >
            Interrupt the current turn and run this now, skipping the queue.
          </Typography>
          <Stack
            direction="row"
            justifyContent="flex-end"
            spacing={1}
            sx={{ mt: 1.5 }}
          >
            <Button
              size="small"
              color="inherit"
              onClick={(): void => setForceAnchor(null)}
            >
              Cancel
              <Kbd keys="Esc" />
            </Button>
            <Button
              size="small"
              variant="contained"
              startIcon={<Bolt />}
              onClick={confirmForce}
            >
              Force push
              <Kbd keys={ENTER_LABEL} />
            </Button>
          </Stack>
        </Box>
      </Popover>
      {compact && (
        <ComposerSheet
          open={sheetOpen}
          onClose={(): void => setSheetOpen(false)}
          session={session}
          options={options}
          loading={showSkeleton}
          dead={dead}
          onSelectOption={(configId, value): void => {
            send({
              type: "set_config_option",
              session_id: sessionId,
              config_id: configId,
              value,
            });
          }}
        />
      )}
      {/* Mobile fullscreen compose (the ↗ on touch). A near-full-screen sheet for
          comfortable long-form writing — the first-class editor + future home of a
          markdown / rich-text toolbar + preview. Shares the composer's `text` +
          attachments; the inline editor is hidden while this is open (xor), so the
          shared editorRef points at the one mounted here. */}
      {composeFs && (
        <DetentSheet
          open
          // Always full-screen frosted glass — the primary mobile writing canvas
          // (does not collapse to content-height when the keyboard is dismissed).
          ariaLabel="Compose message"
          frosted
          cover
          surfaceColor={theme.palette.background.default}
          onClose={(): void => setComposeFs(false)}
          footer={
            <ComposeBar
              dead={dead}
              sendable={sendable}
              options={options}
              showSkeleton={showSkeleton}
              attachments={attachments}
              onRemoveAttachment={removeAttachment}
              onTrigger={(t): void => editorRef.current?.insertTrigger(t)}
              onAttach={(): void => fileInputRef.current?.click()}
              onConfig={(configId, value): void => {
                send({
                  type: "set_config_option",
                  session_id: sessionId,
                  config_id: configId,
                  value,
                });
              }}
              onSend={(): void => {
                submit();
                setComposeFs(false);
              }}
              onCollapse={(): void => setComposeFs(false)}
              onSaveDraft={(): void => {
                saveDraft();
                setComposeFs(false);
              }}
              onForcePush={busy || starting
                ? (anchor): void => setForceAnchor(anchor)
                : undefined}
            />
          }
        >
          <Box sx={{ p: 1.5 }}>
            <ComposerTextarea
              ref={editorRef}
              value={text}
              onChange={setText}
              onSubmit={(): void => {
                submit();
                setComposeFs(false);
              }}
              onSaveDraft={saveDraft}
              sessionId={sessionId}
              commands={(): AvailableCommand[] => availableCommands}
              placeholder="Message the agent…"
              onPasteFiles={addFiles}
              borderless
              expanded
              onEscape={(): boolean => {
                setComposeFs(false);
                return true;
              }}
            />
          </Box>
        </DetentSheet>
      )}
      {
        /* Confirm before stopping a running turn. The Stop button is destructive
          (the in-flight turn ends), so a single click/Esc shouldn't trigger it.
          The Stop action is autoFocused so Enter confirms; Esc dismisses (MUI
          Dialog default), so a stray second Esc backs out rather than cancelling. */
      }
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
            <Kbd keys="Esc" />
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
            <Kbd keys={ENTER_LABEL} />
          </Button>
        </DialogActions>
      </Dialog>
      {
        /* Move-draft destination picker + undo snackbar. Owned here (not in the
          drafts panel) so the snackbar survives when moving the LAST draft
          unmounts that panel. */
      }
      <Sheet
        open={moveSrcId !== null}
        onClose={(): void => setMoveSrcId(null)}
        title="Move draft to…"
      >
        <List sx={{ pb: 1 }}>
          {otherSessions.map((s) => (
            <ListItemButton
              key={s.id}
              onClick={(): void => {
                if (moveSrcId !== null) {
                  moveDraft(sessionId, moveSrcId, s.id);
                  setMoveUndo({ id: moveSrcId, toId: s.id, toTitle: s.title });
                }
                setMoveSrcId(null);
              }}
            >
              <ListItemText
                primary={s.title}
                secondary={s.cwd}
                secondaryTypographyProps={{ noWrap: true }}
              />
            </ListItemButton>
          ))}
        </List>
      </Sheet>
      <Snackbar
        open={moveUndo !== null}
        autoHideDuration={6000}
        onClose={(_e, reason): void => {
          // Keep it up until autohide / Undo — a stray tap (clickaway) shouldn't
          // snatch the Undo away before the user can reach it.
          if (reason !== "clickaway") setMoveUndo(null);
        }}
        anchorOrigin={{ vertical: "top", horizontal: "center" }}
        message={moveUndo ? `Moved to ${moveUndo.toTitle}` : ""}
        action={
          <Button
            color="primary"
            size="small"
            onClick={(): void => {
              if (moveUndo) moveDraft(moveUndo.toId, moveUndo.id, sessionId);
              setMoveUndo(null);
            }}
            sx={{ textTransform: "none" }}
          >
            Undo
          </Button>
        }
      />
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
      {attachments.map((a, i) => (
        <Box
          key={a.id}
          onClick={(): void => openLightbox(attachments, i)}
          sx={{ position: "relative", flexShrink: 0, cursor: "pointer" }}
        >
          {a.isImage && a.previewUrl && !failedIds.has(a.id)
            ? (
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
            )
            : (
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
                <InsertDriveFileOutlined
                  fontSize="small"
                  sx={{ color: "text.secondary", flexShrink: 0 }}
                />
                <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
                  {a.name}
                </Typography>
              </Stack>
            )}
          <IconButton
            aria-label={`remove ${a.name}`}
            size="small"
            // INSIDE the top-right corner (not a negative offset): the strip is
            // `overflowX: auto`, which forces overflow-y to auto too and clipped
            // the old `top:-8` button. A dark scrim keeps it legible over the
            // image; stopPropagation so removing doesn't also open the preview.
            onClick={(e): void => {
              e.stopPropagation();
              onRemove(a.id);
            }}
            sx={{
              position: "absolute",
              top: 3,
              right: 3,
              width: 20,
              height: 20,
              color: "#fff",
              bgcolor: "rgba(0,0,0,0.55)",
              "&:hover": { bgcolor: "rgba(0,0,0,0.72)" },
            }}
          >
            <Close sx={{ fontSize: 13 }} />
          </IconButton>
        </Box>
      ))}
    </Stack>
  );
}

// Read-only attachment preview for a parked draft / queued row: actual mini
// thumbnails (images) + name chips (files), each tapping into the full-screen
// lightbox. A parked message now SHOWS what it carries instead of a blind
// "2 attachments" count. Removal is via editing the row, so no × here.
function QueuedAttachmentChips({
  attachments,
}: {
  attachments: Attachment[];
}): React.JSX.Element {
  return (
    <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5, mt: 0.5 }}>
      {attachments.map((a, i) =>
        a.isImage && a.previewUrl
          ? (
            <Box
              key={a.id}
              component="img"
              src={a.previewUrl}
              alt={a.name}
              onClick={(): void => openLightbox(attachments, i)}
              sx={{
                width: 38,
                height: 38,
                objectFit: "cover",
                borderRadius: 0.75,
                border: 1,
                borderColor: "divider",
                cursor: "pointer",
                display: "block",
              }}
            />
          )
          : (
            <Stack
              key={a.id}
              direction="row"
              spacing={0.5}
              alignItems="center"
              onClick={(): void => openLightbox(attachments, i)}
              sx={{
                height: 38,
                maxWidth: 150,
                px: 0.75,
                borderRadius: 0.75,
                border: 1,
                borderColor: "divider",
                bgcolor: "action.hover",
                color: "text.secondary",
                cursor: "pointer",
              }}
            >
              <InsertDriveFileOutlined sx={{ fontSize: 15, flexShrink: 0 }} />
              <Typography variant="caption" noWrap sx={{ minWidth: 0 }}>
                {a.name}
              </Typography>
            </Stack>
          )
      )}
    </Box>
  );
}

// The Zed-style staging panel above the editor — one component for two kinds:
//   - "queued": prompts the busy agent can't take yet, auto-drained one per turn.
//   - "draft":  parked messages the user holds; activated (sent/queued) on demand.

// Gradient shimmer sweep — Claude "thinking" style — for an optimistic row
// still unconfirmed past SHIMMER_DELAY_MS (see store's optimisticDrafts).
const sweep = keyframes`to { background-position: -200% 0; }`;

// An OPTIMISTIC draft row: shown the instant you stage it, before the daemon
// confirms. `pending` (<200ms) renders like a normal row so a fast LAN/tailnet
// send never flashes a loader; `sending` shimmers in the theme colour; `failed`
// (WS down / timed out) turns red with retry + discard. Reconciled out of the
// store by cmid the moment its confirmed twin arrives, so it never duplicates.
function OptimisticDraftRow({
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
    <Paper
      elevation={0}
      sx={(t) => ({
        p: 0.75,
        display: "flex",
        alignItems: "flex-start",
        gap: 0.5,
        bgcolor: failed
          ? alpha(t.palette.error.main, 0.06)
          : sending
          ? alpha(t.palette.primary.main, 0.05)
          : "background.paper",
        // Coloured leading edge marks the row's state at a glance: red = failed,
        // primary = in flight (mirrors the failed affordance so "sending" reads
        // as clearly as "failed", not as a near-invisible text shimmer).
        ...(failed && { borderLeft: `3px solid ${t.palette.error.main}` }),
        ...(sending && !failed &&
          { borderLeft: `3px solid ${t.palette.primary.main}` }),
      })}
    >
      {sending && (
        <CircularProgress
          size={13}
          thickness={5}
          sx={{ color: "primary.main", mt: 0.25, flexShrink: 0 }}
        />
      )}
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Typography
          variant="body2"
          sx={(t) => ({
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
            display: "-webkit-box",
            WebkitLineClamp: 2,
            WebkitBoxOrient: "vertical",
            overflow: "hidden",
            ...(sending && {
              background:
                `linear-gradient(90deg, ${t.palette.text.secondary} 0%, ${t.palette.text.secondary} 35%, ${t.palette.primary.main} 50%, ${t.palette.text.secondary} 65%, ${t.palette.text.secondary} 100%)`,
              backgroundSize: "200% 100%",
              WebkitBackgroundClip: "text",
              backgroundClip: "text",
              color: "transparent",
              animation: `${sweep} 2s linear infinite`,
              "@media (prefers-reduced-motion: reduce)": { animation: "none" },
            }),
          })}
        >
          {message.text || "📎 attachment"}
        </Typography>
        {failed && (
          <Typography variant="caption" sx={{ color: "error.main" }}>
            Failed to send
          </Typography>
        )}
        {message.attachments.length > 0 && (
          <QueuedAttachmentChips attachments={message.attachments} />
        )}
      </Box>
      {failed && (
        <Stack direction="row" sx={{ flexShrink: 0 }}>
          <Tooltip title="Retry">
            <IconButton
              size="small"
              aria-label="retry send"
              onClick={(): void => retryQueued(sessionId, cmid)}
            >
              <Refresh fontSize="small" />
            </IconButton>
          </Tooltip>
          <Tooltip title="Discard">
            <IconButton
              size="small"
              aria-label="discard"
              onClick={(): void => discardQueued(sessionId, cmid)}
            >
              <Close fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>
      )}
    </Paper>
  );
}

// A header action that confirms before firing — the bulk panel actions (Clear
// All, Send all) are one tap from wiping or dispatching the whole list, so they
// route through a small confirm Popover (same pattern as the row's Force-push
// confirm) instead of acting instantly.
function ConfirmButton({
  label,
  message,
  confirmLabel,
  confirmColor,
  color = "inherit",
  muted = false,
  onConfirm,
}: {
  label: string;
  message: string;
  confirmLabel: string;
  confirmColor: "primary" | "error" | "warning";
  color?: "inherit" | "primary";
  muted?: boolean;
  onConfirm: () => void;
}): React.JSX.Element {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const confirm = (): void => {
    onConfirm();
    setAnchor(null);
  };
  useConfirmEnter(anchor !== null, confirm);
  return (
    <>
      <Button
        size="small"
        color={color}
        onClick={(e): void => setAnchor(e.currentTarget)}
        sx={{
          textTransform: "none",
          minWidth: 0,
          px: 0.75,
          ...(muted && { color: "text.secondary" }),
        }}
      >
        {label}
      </Button>
      <Popover
        open={anchor !== null}
        anchorEl={anchor}
        onClose={(): void => setAnchor(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: "top", horizontal: "right" }}
      >
        <Box sx={{ p: 1.5, maxWidth: 240 }}>
          <Typography variant="body2" sx={{ mb: 1 }}>
            {message}
          </Typography>
          <Stack direction="row" spacing={1} justifyContent="flex-end">
            <Button
              size="small"
              color="inherit"
              onClick={(): void => setAnchor(null)}
              sx={{ textTransform: "none" }}
            >
              Cancel
              <Kbd keys="Esc" />
            </Button>
            <Button
              size="small"
              variant="contained"
              color={confirmColor}
              onClick={confirm}
              sx={{ textTransform: "none" }}
            >
              {confirmLabel}
              <Kbd keys={ENTER_LABEL} />
            </Button>
          </Stack>
        </Box>
      </Popover>
    </>
  );
}

// Collapsible header ("N Queued Messages" / "N Drafts" + Clear All, plus Send all
// for drafts) over a scroll-capped list of rows. Drafts sit BELOW the queue and
// above the composer (see the Composer render).
function PendingPanel({
  kind,
  sessionId,
  items,
  status,
  commands,
  onMoveDraft,
}: {
  kind: "queued" | "draft";
  sessionId: string;
  items: QueuedMessage[];
  /** Session status — drives Send-now vs Force-push (queued) / Send vs Queue (draft). */
  status: Status;
  /** Agent-advertised `/` commands, threaded into the row's inline editor. */
  commands: () => AvailableCommand[];
  /** Open the "move to another session" picker for a draft (draft kind only).
      Owned by the Composer so it survives this panel unmounting when the last
      draft leaves. Absent → the row's kebab omits the Move action. */
  onMoveDraft?: ((id: string) => void) | undefined;
}): React.JSX.Element {
  // Collapsed state is an APP-LEVEL (per-device) UI pref, NOT service state: it
  // persists in localStorage per panel kind so it survives reloads + session
  // switches, but is never synced across terminals (mirrors PlanDock's
  // `cowboy:plan-expanded`). Default expanded; the count stays visible either way.
  const collapse = collapseStore(`cowboy:${kind}-collapsed`);
  const collapsed = usePrefStore(collapse);
  const toggleCollapsed = (): void => {
    collapse.set(!collapsed);
  };
  const [editingId, setEditingId] = useState<string | null>(null);
  // Reorder is a low-frequency action, so the per-row drag grips are hidden by
  // default (they'd waste ~40px on every row of a narrow phone) and revealed
  // only in this opt-in "reorder mode" (iOS list-Edit pattern). Local + ephemeral
  // like `collapsed`. Per-panel state → drafts and queue toggle independently.
  const [reordering, setReordering] = useState(false);
  const count = items.length;
  // Reordering 0/1 items is meaningless — drop out of the mode (and hide its
  // toggle) so a cleared/sent-down panel never sits stuck in an empty mode.
  useEffect(() => {
    if (count < 2 && reordering) setReordering(false);
  }, [count, reordering]);
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
      kind === "queued"
        ? reorderQueue(sessionId, order)
        : reorderDrafts(sessionId, order),
    onDragStart: kind === "queued"
      ? (): void => {
        const head = items[0];
        if (head) setQueueEditing(sessionId, head.id);
      }
      : undefined,
    onDragEnd: kind === "queued"
      ? (): void => setQueueEditing(sessionId, null)
      : undefined,
  });
  const noun = kind === "queued" ? "Queued Message" : "Draft";
  return (
    <Box
      sx={{
        mb: 1,
        // The original framed container, KEPT (the user liked it): a soft tinted,
        // rounded, bordered box that groups the rows. Its OUTER edge sits at the
        // composer's content gutter — exactly where the input box's outer border is
        // — so the panel frame and the message box line up edge-to-edge (no horizontal
        // margin on either). Drafts read a touch more "staging" than the live queue.
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        bgcolor: kind === "draft" ? "action.selected" : "action.hover",
        overflow: "hidden",
        // Query container for the rows: their secondary actions go inline on a
        // roomy panel (iPad / desktop) and collapse to a kebab on a narrow phone
        // one (ROW_ACTIONS_INLINE). Keyed on the ACTUAL panel width, so the
        // desktop sidebar drag / portal iframe / orientation all resolve right —
        // a viewport breakpoint can't, since iPad shares the phone "mobile" bucket.
        containerType: "inline-size",
        containerName: "pendingPanel",
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        // Pin the header to the SAME 44px as the composer input (ComposerTextarea
        // `MuiInputBase-root` minHeight) so the "N Drafts" bar and the message box
        // read as the same-height pair. `py: 0` drops the old extra 8px that made
        // the bar (a 44px icon button + padding) taller than the input.
        sx={{ pl: 0.5, pr: 0.75, py: 0, minHeight: 44 }}
      >
        <IconButton
          size="small"
          aria-label={collapsed ? "expand" : "collapse"}
          onClick={toggleCollapsed}
          sx={{ flexShrink: 0 }}
        >
          {collapsed
            ? <ChevronRight fontSize="small" />
            : <ExpandMore fontSize="small" />}
        </IconButton>
        <Typography
          variant="caption"
          sx={{ fontWeight: 600, flex: 1, minWidth: 0, cursor: "pointer" }}
          onClick={toggleCollapsed}
        >
          {count} {noun}
          {count === 1 ? "" : "s"}
        </Typography>
        {
          /* Reorder toggle — reveals the per-row drag grips. Only meaningful (and
            only shown) with 2+ rows. Primary-tinted while active. HIDDEN on a wide
            panel (ROW_ACTIONS_INLINE): there the grips are always shown, so the
            toggle is redundant — same adaptive rule as the row actions. */
        }
        {count >= 2 && (
          <IconButton
            size="small"
            aria-label={reordering ? "done reordering" : "reorder"}
            title={reordering ? "Done" : "Reorder"}
            color={reordering ? "primary" : "default"}
            onClick={(): void => setReordering((r) => !r)}
            sx={{ flexShrink: 0, [ROW_ACTIONS_INLINE]: { display: "none" } }}
          >
            <SwapVert fontSize="small" />
          </IconButton>
        )}
        {kind === "draft" && (
          <ConfirmButton
            label="Send all"
            message={`Send all ${String(count)} drafts to the agent?`}
            confirmLabel="Send all"
            confirmColor="primary"
            color="primary"
            onConfirm={(): void => activateAllDrafts(sessionId)}
          />
        )}
        <ConfirmButton
          label="Clear All"
          message={kind === "queued"
            ? `Clear all ${String(count)} queued messages?`
            : `Clear all ${String(count)} drafts?`}
          confirmLabel="Clear all"
          confirmColor="error"
          muted
          onConfirm={(): void => {
            if (kind === "queued") clearQueue(sessionId);
            else clearDrafts(sessionId);
          }}
        />
      </Stack>
      <Collapse in={!collapsed}>
        <Stack
          spacing={0.5}
          sx={{
            // Inner padding so the rows sit INSIDE the frame with a small inset
            // (the original framed look). The frame's OUTER edge is what aligns
            // with the input box, not the rows.
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
            // A LOCAL optimistic draft (carries `status`) renders a lightweight
            // row with no grip / edit / reorder — it isn't a server item yet.
            const optimistic = m.status !== undefined;
            return (
              <Stack
                key={m.id}
                ref={sortable.registerItem(m.id)}
                style={sortable.itemStyle(m.id)}
                direction="row"
                alignItems="center"
                spacing={0.5}
              >
                {
                  /* Leading grip — visibility is ADAPTIVE: on a narrow panel it's
                    hidden until reorder mode (so rows reclaim ~40px), but on a wide
                    panel (ROW_ACTIONS_INLINE) it's always shown — there's room, so
                    no reorder toggle is needed (the toggle hides itself there too).
                    Always RENDERED when draggable so CSS alone decides; never on the
                    row being edited (the edit field owns it) nor an optimistic row. */
                }
                {editingId !== m.id && !optimistic && (
                  <IconButton
                    {...sortable.handleProps(m.id)}
                    aria-label="Drag to reorder"
                    sx={{
                      ...TOOLBAR_ICON_BTN,
                      color: "text.disabled",
                      display: reordering ? "inline-flex" : "none",
                      [ROW_ACTIONS_INLINE]: { display: "inline-flex" },
                    }}
                  >
                    <DragIndicator fontSize="small" />
                  </IconButton>
                )}
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  {optimistic
                    ? <OptimisticDraftRow sessionId={sessionId} message={m} />
                    : (
                      <PendingRow
                        kind={kind}
                        sessionId={sessionId}
                        message={m}
                        status={status}
                        commands={commands}
                        editing={editingId === m.id}
                        onEdit={(): void => setEditingId(m.id)}
                        onEditDone={(): void => setEditingId(null)}
                        onMove={onMoveDraft
                          ? (): void => onMoveDraft(m.id)
                          : undefined}
                      />
                    )}
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
// Plain-text preview that clamps to 2 lines, with a Show more / Show less toggle
// at the end that appears ONLY when the text actually overflows — so a long
// pasted draft / queued prompt stays a compact row by default but expands inline
// (the "多行末尾要有折叠按钮，默认折叠" ask). Measured only while clamped (the
// expanded state has no overflow to read) and re-measured on resize. The toggle
// is a real button with a touch-sized hit target (mobile + desktop).
function ClampedText({ text, onTextClick }: { text: string; onTextClick?: () => void }): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || expanded) return undefined;
    const measure = (): void =>
      setOverflowing(el.scrollHeight > el.clientHeight + 1);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [text, expanded]);
  return (
    <>
      <Box
        ref={ref}
        onClick={onTextClick}
        sx={{
          typography: "body2",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          // Tap the text to edit the message (the "Show more" button below has
          // its own handler and isn't affected).
          ...(onTextClick && { cursor: "pointer" }),
          ...(expanded ? {} : {
            display: "-webkit-box",
            WebkitLineClamp: 2,
            WebkitBoxOrient: "vertical",
            overflow: "hidden",
          }),
        }}
      >
        {text}
      </Box>
      {(overflowing || expanded) && (
        <Button
          size="small"
          disableRipple
          onClick={(): void => setExpanded((e) => !e)}
          endIcon={
            <ExpandMore
              fontSize="small"
              sx={{
                transition: "transform .15s",
                transform: expanded ? "rotate(180deg)" : "none",
              }}
            />
          }
          sx={{
            mt: 0.25,
            px: 0.5,
            py: 0,
            minWidth: 0,
            minHeight: 28,
            "@media (pointer: coarse)": { minHeight: 32 },
            textTransform: "none",
            fontSize: "0.72rem",
            color: "text.secondary",
            "& .MuiButton-endIcon": { ml: 0.25 },
            "&:hover": { bgcolor: "transparent", color: "text.primary" },
          }}
        >
          {expanded ? "Show less" : "Show more"}
        </Button>
      )}
    </>
  );
}

// right now: dispatchable → a plain "Send now" (sends immediately, revives a
// dead session); busy → a warning-coloured "Force push" that interrupts the
// running turn and runs this prompt next — gated behind a confirm popover
// because cancelling discards the in-flight turn's progress.

// Container-query breakpoint for a row's secondary actions (see PendingPanel's
// containerName). Above this panel width the row shows every action inline; below
// it they fold into the kebab. ~520px: comfortably fits Send + 3 icons + readable
// text on an iPad/desktop panel, while every portrait phone panel (≤ ~410px) stays
// kebab. A landscape phone (panel ~900px) genuinely has the room, so inline there
// is correct too — exactly what keying on real width (not device class) buys.
const ROW_ACTIONS_INLINE = "@container pendingPanel (min-width: 520px)";

function PendingRow({
  kind,
  sessionId,
  message,
  status,
  commands,
  editing,
  onEdit,
  onEditDone,
  onMove,
}: {
  kind: "queued" | "draft";
  sessionId: string;
  message: QueuedMessage;
  status: Status;
  commands: () => AvailableCommand[];
  editing: boolean;
  onEdit: () => void;
  onEditDone: () => void;
  /** Open the move-to-another-session picker for this row (draft kind only). */
  onMove?: (() => void) | undefined;
}): React.JSX.Element {
  const [draft, setDraft] = useState(message.text);
  // Per-row kebab (⋮) anchor — holds the draft's secondary actions (Edit / Move
  // / Remove) so the row shows only Send inline and stays uncluttered.
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null);
  // Local attachments while editing, seeded from the queued message. The edit
  // box is the SAME ComposerEditor as the main composer, so a queued prompt can
  // gain/lose images here too (pasted screenshots, picked files).
  const [editAttachments, setEditAttachments] = useState<Attachment[]>(
    message.attachments,
  );
  const editorRef = useRef<ComposerEditorHandle>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  // Confirm popover for force push (anchored to the Bolt button). Null = closed.
  const [confirmAnchor, setConfirmAnchor] = useState<HTMLElement | null>(null);
  const confirmForcePush = (): void => {
    forcePushQueued(sessionId, message.id);
    setConfirmAnchor(null);
  };
  useConfirmEnter(confirmAnchor !== null, confirmForcePush);
  // Per-row delete confirm. Dropping a queued message / draft is irreversible, so
  // the × opens this modal instead of deleting on a single tap.
  const [confirmRemove, setConfirmRemove] = useState(false);
  const doRemove = (): void => {
    if (kind === "draft") removeDraft(sessionId, message.id);
    else removeQueued(sessionId, message.id);
    setConfirmRemove(false);
  };
  useConfirmEnter(confirmRemove, doRemove);
  // "running" is the idle-ready state; "exited"/"crashed"/"interrupted" dispatch
  // a revive. Anything else ("busy"/"starting") has an in-flight turn → force push.
  const dispatchable = status === "running" ||
    status === "exited" ||
    status === "crashed" ||
    status === "interrupted";
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
  // Focused edit overlay (Zed-style ↗): a near-full-screen DetentSheet hosting a
  // tall editor so a long queued message edits comfortably without ballooning the
  // queue panel inline. ONE editor is mounted at a time (inline XOR overlay), both
  // driving the shared `draft` — so there's no uncontrolled-editor desync.
  const theme = useTheme();
  const [overlayOpen, setOverlayOpen] = useState(false);
  const overlayEditorRef = useRef<ComposerEditorHandle>(null);
  useLayoutEffect(() => {
    if (!overlayOpen) return undefined;
    // Small delay so the sheet has mounted before we focus (desktop pops the
    // caret; on touch the keyboard may need one tap — acceptable for an edit).
    const t = globalThis.setTimeout(() => overlayEditorRef.current?.focusEnd(), 60);
    return () => globalThis.clearTimeout(t);
  }, [overlayOpen]);
  useLayoutEffect(() => {
    if (!editing) return undefined;
    // On TOUCH, editing a queued/draft message goes straight to the fullscreen
    // overlay — no cramped inline edit on a phone (the mobile fullscreen-first
    // design). Desktop keeps the inline edit + the ↗ to expand.
    if (touchInput) {
      // Raise the keyboard IN this gesture's tick (the editing→overlay layout
      // effect runs synchronously in the Edit tap's task, before paint — same
      // window the inline focus below relies on). The overlay's later focusEnd
      // only transfers focus between inputs; iOS won't raise the keyboard from
      // that passive timer, so without this the fullscreen edit opened with the
      // keyboard DOWN (it should default to the typing state, like compose).
      claimKeyboard();
      setOverlayOpen(true);
      return undefined;
    }
    // focusEnd, not focus: opening an existing draft/queued message should put
    // the caret at the end of its text so you continue typing, not at the start.
    editorRef.current?.focusEnd();
    // Focusing raises the keyboard; with `interactive-widget=resizes-content` the
    // layout viewport then shrinks. A row low in the drafts list would land BEHIND
    // the keyboard (you'd have to scroll up to see what you're typing). Once the
    // keyboard has settled, pull the edit row into view.
    const t = globalThis.setTimeout(() => {
      rowRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
    }, 350);
    return () => globalThis.clearTimeout(t);
  }, [editing, touchInput]);
  if (editing) {
    const save = (): void => {
      if (kind === "draft") {
        editDraft(sessionId, message.id, draft, editAttachments);
      } else editQueued(sessionId, message.id, draft, editAttachments);
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
      <>
        <Paper ref={rowRef} variant="outlined" sx={{ p: 0.75 }}>
          {editAttachments.length > 0 && (
            <AttachmentPreviews
              attachments={editAttachments}
              onRemove={(id): void =>
                setEditAttachments((prev) => prev.filter((a) => a.id !== id))}
            />
          )}
          {/* Inline editor — hidden while the focused overlay owns the edit so only
              ONE editor is mounted at a time (shared `draft`, no uncontrolled desync). */}
          {!overlayOpen &&
            (touchInput
              ? (
                <ComposerTextarea
                  ref={editorRef}
                  value={draft}
                  borderless
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
              )
              : (
                <ComposerEditor
                  ref={editorRef}
                  // Seeds from the shared `draft` and re-mounts on overlay close, so
                  // it reflects edits made in the overlay. Uncontrolled thereafter —
                  // onChange feeds `draft`, never back into `value`.
                  value={draft}
                  borderless
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
              ))}
          <Stack
            direction="row"
            alignItems="center"
            spacing={0.5}
            sx={{ mt: 0.5 }}
          >
            <Tooltip title="Expand editor">
              <span>
                <IconButton
                  size="small"
                  aria-label="expand editor"
                  onClick={(): void => setOverlayOpen(true)}
                >
                  <OpenInFull fontSize="small" />
                </IconButton>
              </span>
            </Tooltip>
            <Box sx={{ flex: 1 }} />
            <Button
              size="small"
              color="inherit"
              onClick={cancel}
              sx={{ textTransform: "none" }}
            >
              Cancel
            </Button>
            <Button
              size="small"
              variant="contained"
              onClick={save}
              sx={{ textTransform: "none" }}
            >
              Save
            </Button>
          </Stack>
        </Paper>
        {/* Focused edit overlay (Step 7): a near-full-screen frosted sheet hosting a
            tall editor for comfortable long-message editing without ballooning the
            queue panel. Save/Cancel end the row edit; grab-dismiss returns to the
            inline box (the shared `draft` is preserved). */}
        {overlayOpen && (
          <DetentSheet
            open
            // Always full-screen frosted glass — matches the compose sheet.
            ariaLabel="Edit message"
            frosted
            cover
            surfaceColor={theme.palette.background.default}
            // Desktop: dismiss returns to the inline edit (the overlay is an
            // optional expand). Touch: the overlay IS the edit (no inline card on a
            // phone), so a grab-dismiss abandons the edit rather than dropping to a
            // cramped inline box.
            onClose={touchInput
              ? cancel
              : (): void => setOverlayOpen(false)}
            header={
              // px matches the body's p:1.5 so the title's left edge lines up with
              // the attachments + editor below (it was flush to the edge before);
              // pb gives it breathing room under the grab handle.
              <Box sx={{ px: 1.5, pb: 0.5 }}>
                <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
                  Edit message
                </Typography>
              </Box>
            }
            footer={
              // Same docked bar as the compose sheet (the user asked Edit to match
              // the input): Send saves the edit; grab-dismiss / the close ×
              // cancels (onClose → cancel). No Cancel/Save text buttons. Config
              // chips are session-level (not available per-row here), so the edit
              // bar is just the triggers + Send.
              <ComposeBar
                dead={false}
                sendable={!!draft.trim() || editAttachments.length > 0}
                attachments={editAttachments}
                onRemoveAttachment={(id): void =>
                  setEditAttachments((prev) => prev.filter((a) => a.id !== id))}
                onTrigger={(t): void => overlayEditorRef.current?.insertTrigger(t)}
                onSend={(): void => {
                  save();
                  setOverlayOpen(false);
                }}
              />
            }
          >
            <Box sx={{ p: 1.5 }}>
              <ComposerEditor
                ref={overlayEditorRef}
                value={draft}
                borderless
                expanded
                onChange={setDraft}
                onSubmit={(): void => {
                  save();
                  setOverlayOpen(false);
                }}
                sessionId={sessionId}
                commands={commands}
                placeholder="Edit message…"
                onPasteFiles={addEditFiles}
                onEscape={(): boolean => {
                  setOverlayOpen(false);
                  return true;
                }}
              />
            </Box>
          </DetentSheet>
        )}
      </>
    );
  }
  // Secondary actions (everything but the primary Send / Force). Defined once,
  // rendered two ways: inline icons on a roomy panel, the same list as kebab
  // MenuItems on a narrow one — toggled purely by the ROW_ACTIONS_INLINE
  // container query (no JS measurement, no duplicate logic).
  const secondary: {
    key: string;
    label: string;
    icon: React.JSX.Element;
    onClick: () => void;
  }[] = kind === "draft"
    ? [
      {
        key: "edit",
        label: "Edit",
        icon: <EditOutlined fontSize="small" />,
        onClick: onEdit,
      },
      ...(onMove
        ? [{
          key: "move",
          label: "Move to another session…",
          icon: <DriveFileMoveOutlined fontSize="small" />,
          onClick: onMove,
        }]
        : []),
      {
        key: "remove",
        label: "Remove",
        icon: <Close fontSize="small" />,
        onClick: (): void => setConfirmRemove(true),
      },
    ]
    : [
      {
        key: "return",
        label: "Return to drafts",
        icon: <Undo fontSize="small" />,
        onClick: (): void => queuedToDraft(sessionId, message.id),
      },
      {
        key: "edit",
        label: "Edit",
        icon: <EditOutlined fontSize="small" />,
        onClick: onEdit,
      },
      {
        key: "remove",
        label: "Remove",
        icon: <Close fontSize="small" />,
        onClick: (): void => setConfirmRemove(true),
      },
    ];

  // Primary action — always inline. Drafts always Send (send-or-queue); a queued
  // row Sends now when the session's free, else Force-pushes (confirm popover).
  // Built as a statement to avoid a nested ternary in the JSX.
  let primary: React.JSX.Element;
  if (kind === "draft") {
    primary = (
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
    );
  } else if (dispatchable) {
    primary = (
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
    );
  } else {
    primary = (
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
    );
  }
  return (
    <Paper
      variant="outlined"
      sx={{ p: 0.75, display: "flex", alignItems: "flex-start", gap: 0.5 }}
    >
      <Box sx={{ flex: 1, minWidth: 0 }}>
        {message.text && <ClampedText text={message.text} onTextClick={onEdit} />}
        {message.attachments.length > 0 && (
          <QueuedAttachmentChips attachments={message.attachments} />
        )}
      </Box>
      <Stack direction="row" alignItems="center" sx={{ flexShrink: 0 }}>
        {primary}
        {/* Force-push confirm — only the queued row has the force path. */}
        {kind === "queued" && (
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
                  <Kbd keys="Esc" />
                </Button>
                <Button
                  size="small"
                  variant="contained"
                  color="warning"
                  startIcon={<Bolt />}
                  onClick={confirmForcePush}
                  sx={{ textTransform: "none" }}
                >
                  Force push
                  <Kbd keys={ENTER_LABEL} />
                </Button>
              </Stack>
            </Box>
          </Popover>
        )}
        {/* Secondary actions — inline on a roomy (iPad/desktop) panel … */}
        <Stack
          direction="row"
          alignItems="center"
          sx={{ display: "none", [ROW_ACTIONS_INLINE]: { display: "flex" } }}
        >
          {secondary.map((a) => (
            <Tooltip key={a.key} title={a.label}>
              <IconButton size="small" aria-label={a.label} onClick={a.onClick}>
                {a.icon}
              </IconButton>
            </Tooltip>
          ))}
        </Stack>
        {/* … and collapsed into a kebab on a narrow (phone) panel. */}
        <Box sx={{ [ROW_ACTIONS_INLINE]: { display: "none" } }}>
          <Tooltip title="More">
            <IconButton
              size="small"
              aria-label={kind === "draft"
                ? "draft actions"
                : "message actions"}
              onClick={(e): void => setMenuAnchor(e.currentTarget)}
            >
              <MoreVert fontSize="small" />
            </IconButton>
          </Tooltip>
          <Menu
            anchorEl={menuAnchor}
            open={menuAnchor !== null}
            onClose={(): void => setMenuAnchor(null)}
          >
            {secondary.map((a) => (
              <MenuItem
                key={a.key}
                onClick={(): void => {
                  setMenuAnchor(null);
                  a.onClick();
                }}
              >
                <ListItemIcon>{a.icon}</ListItemIcon>
                <ListItemText>{a.label}</ListItemText>
              </MenuItem>
            ))}
          </Menu>
        </Box>
      </Stack>
      <Dialog
        open={confirmRemove}
        onClose={(): void => setConfirmRemove(false)}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>
          {kind === "draft"
            ? "Delete this draft?"
            : "Delete this queued message?"}
        </DialogTitle>
        {message.text && (
          <DialogContent>
            <DialogContentText
              sx={{
                display: "-webkit-box",
                WebkitLineClamp: 3,
                WebkitBoxOrient: "vertical",
                overflow: "hidden",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
              }}
            >
              {message.text}
            </DialogContentText>
          </DialogContent>
        )}
        <DialogActions>
          <Button
            color="inherit"
            onClick={(): void => setConfirmRemove(false)}
            sx={{ textTransform: "none" }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <Button
            color="error"
            variant="contained"
            autoFocus
            onClick={doRemove}
            sx={{ textTransform: "none" }}
          >
            Delete
            <Kbd keys={ENTER_LABEL} />
          </Button>
        </DialogActions>
      </Dialog>
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
    <Sheet open={open} onClose={onClose} forceSheet={navbarAtBottom}>
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
            {loading
              ? (
                <Stack
                  direction="row"
                  spacing={1.5}
                  alignItems="center"
                  sx={{ py: 1, color: "text.secondary" }}
                >
                  <CircularProgress size={16} />
                  <Typography variant="body2">
                    Loading agent options…
                  </Typography>
                </Stack>
              )
              : (
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
    </Sheet>
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
      if (
        u.sessionUpdate === "available_commands_update" &&
        Array.isArray(u.availableCommands)
      ) {
        return u.availableCommands;
      }
    }
  }
  return [];
}
