import { desktopSize, glyphSize } from "./surface/desktopSize";
import { DeliveryPicker } from "./DeliveryPicker";
import { prefetchDeliveryOptions } from "./deliveryPolicy";
import { SendDeliveryIcon } from "./SendDeliveryIcon";

import { QueueDeliverySettings } from "./QueueDeliverySettings";
import type { DeliveryTarget } from "./currentTurnDelivery";

import { DraftDestinationPicker } from "./DraftDestinationPicker";

import { lazy, type PointerEvent as ReactPointerEvent, type ReactNode, Suspense, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { alpha, Badge, Box, Button, ButtonBase, Chip, CircularProgress, ClickAwayListener, Collapse, DialogContentText, Divider, IconButton, LinearProgress, List, MenuItem, Paper, Popper, Snackbar, Stack, Switch, TextField, Tooltip, Typography, useMediaQuery, useTheme } from "@mui/material";
import { AlternateEmail, AttachFile, Check, CleaningServices, Close, CloseFullscreen, Compress, DeleteOutline, EditNoteOutlined, ExpandMore, KeyboardHide, OpenInFull, Refresh, Schedule, Stop, Tune, VerticalAlignBottom, Visibility } from "@mui/icons-material";
import {
  type ComposerEditorHandle,
  type ComposerEditorSelection,
  PlatformComposerEditor,
} from "./composer/PlatformComposerEditor";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { useComposerDraftController } from "./composer/useComposerDraftController";
import { useComposerFileDrop } from "./composer/composerFileDrop";

import { didMobileSoftwareKeyboardClose, dismissMobileSoftwareKeyboard, beginMobileEditorFocusTransfer, isMobileEditorFocusTransferPending, releaseMobileComposerFocus, shouldPresentMobileKeyboardSurface, noteMobileKeyboardDismissed, clearMobileKeyboardDismissed } from "./composer/mobileComposerFocus";
import { useKeyboardOpen } from "./keyboardInset";
import { attachmentTrayForSurface } from "./composer/attachmentPresentation";

import type { ComposerWorkspaceProps } from "./composer/contracts";
import { resolveSessionAction, type SessionAction } from "./agentCommands";
import { ConfirmConsequence } from "./ConfirmConsequence";
import {
  currentProviderEntry,
  useProviderCatalog,
} from "./providerCatalog";
import { providerName } from "./providerPresentation";
import { SessionProviderAccess } from "./ProviderManagement";
import { SessionAccountDetails } from "./SessionAccountDetails";
import { resolveProviderAuthenticationPresentation } from "@cowboy/provider-ui";
import {
  sessionProviderFacts,
  sessionProviderManageLabel,
  sessionProviderNeedsAttention,
  sessionProviderShowsUsage,
  sessionProviderSummary,
  sessionProviderUsageEmptyMessage,
  sessionProviderUsageRows,
  workspaceOptionsSummary,
} from "./sessionSettingsPresentation";
import {
  providerUsage,
  providerUsageAccount,
  usageRefreshing,
} from "./usageLimits";
import { expectHttpOk } from "./httpResponse";
import { SessionReloadDialog } from "./SessionReloadDialog";
import { ProviderUpdateCard } from "./ProviderUpdateCard";
import { ProviderUpdateDialog } from "./ProviderUpdateDialog";
import { sessionProviderUpdate } from "./providerUpdateOffer";
import { createPortal, flushSync } from "react-dom";
import { FullscreenComposer } from "./FullscreenComposer";
import { ComposerToolbarSettings } from "./ComposerToolbarSettings";
import { useComposerToolbar } from "./composerToolbarConfig";
import { MobileComposerFormatActions } from "./MobileComposerFormatActions";
import { MobileComposerAccessoryButton, MobileComposerEditingBar, MobileComposerFixedActionSlot } from "./MobileComposerAccessoryDock";

import { dismissPromptSuggestion, usePromptSuggestion } from "./promptSuggestions";
import { useTouchComposer } from "./ComposerTextarea";
import { shouldExpandInlineComposer } from "./composer/mobileCompactEditorPolicy";

import { isImeKeyEvent } from "./imeKey";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";

import { openLightbox } from "./ResourceLightbox";
import { PlanDock } from "./PlanDock";
import { ManagedCallsDock } from "./ManagedCallsDock";
import { BackgroundTasksBar } from "./BackgroundTasksBar";
import { useCallsDockLegacy } from "./backgroundTasks";
import { SessionToolsSection } from "./AgentToolsPanel";
import { SessionContextSettings } from "./SessionContextSettings";
import { isManagedChild } from "./managedCalls";
import { mobileComposerFocusMotion, mobileComposerIdleEditorMinHeight, mobileComposerKeyboardGap, mobileComposerPanelFrameSx, mobileComposerStackGap } from "./mobileComposerPrimitives";
import { mobileComposerHairlineColor, mobileFocusedComposerFill, mobileFocusedComposerSurfaceSx } from "./mobileComposerSurface";
import { TurnStatusOverlay } from "./TurnStatusOverlay";
import { ProviderQuotaStatus } from "./ProviderQuotaStatus";
import {
  SessionContextCue,
  SessionHandedOffDock,
  useHandedOff,
  useAnnouncedSessionContext,
} from "./SessionContextCue";
import { PermissionOverlay } from "./PermissionOverlay";
import { useApprovalDockOwns } from "./approvalDockStore";
import { CallApprovalOverlay } from "./CallApprovalOverlay";
import { useCallApproval } from "./callApproval";
import {
  composerTimelineSlice,
  sameComposerTimelineSlice,
} from "./composerTimelineSlice";
import {
  composerSessionSlice,
  sameComposerSessionSlice,
  sameComposerSheetSession,
} from "./composerSessionSlice";
import {
  setComposerExpanded,
  setComposerHeight,
  toggleComposerExpanded,
  useComposerExpanded,
  useComposerHeight,
} from "./composerExpand";
import { setVimMode } from "./vimModeStore";
import { requestStickToBottom, setSticky, useSticky } from "./stickyStore";
import { useVimSetting } from "./vimSetting";
import { useCompactionContext } from "./useCompactionContext";
import {
  type TranscriptProjection,
  useExploreAtTail,
} from "./explore/exploreStore";
import { DESKTOP_FOCUS_CALLS_SHORTCUT, DESKTOP_FOCUS_PLAN_SHORTCUT, DESKTOP_FOCUS_PROMPT_SHORTCUT } from "./desktop/commands/workspaceShortcuts";
import { type Attachment } from "./attachments";
import { getInlineAttachment, setImageTapHandler } from "./inlineImages";

import { cancelTurn, moveDraft, openSession, type QueuedMessage, renameSession, resetSession, scheduleDraft, send, setSessionConfigOptions, setPaused, pauseQueueConfirmed, loadUsageSnapshot, requestUsageRefresh, submitPrompt, unscheduleDraft, useConnected, useStoreSelector } from "./store";
import { setActiveSessionId } from "./controlPlane";
import { ScheduleSheet } from "./ScheduleSheet";

import { haptic, importantHaptic, navigationHaptic } from "./haptic";

import { useNavbarAtBottom } from "./navbarSettings";
import { useReadingSettings } from "./readingSettings";
import {
  useReliableTouchTap,
  useRetargetedTouchClickGuard,
} from "./useReliableTouchTap";
import {
  NetworkButton,
  NetworkIconButton,
  useNetworkActionState,
} from "./NetworkActionFeedback";
import type { ConfigOptionChange } from "./configOptionMutation";
import { originLabel } from "./protocol";
import {
  providerConfigOptionDisabled,
  providerConfigOptionOrder,
  providerConfigOptionPresentations,
  providerConfigOptions,
} from "./providerConfigOptions";
import {
  activeRunConfigPreset,
  type RunConfigPreset,
  runConfigCurrentTitle,
  runConfigPresetChanges,
  runConfigPresets,
  runConfigSummary,
} from "./runConfigPresets";
import {
  usageCacheIntervalLabel,
  usageCacheMinHitTokens,
  usageCacheOptionName,
} from "./usageHostMap";
import type {
  AvailableCommand,
  ConfigOption,
  Delivery,
  DraftSchedule,
  SessionMeta,
  Status,
} from "./protocol";
import { sessionProjectLabel } from "./sessionProject";
import { sessionExecution } from "./sessionExecution";
import { ConfirmSheet, Sheet } from "./Sheet";
import { MobileSheetDismiss } from "@cowboy/app-shell";
import {
  CLOSE_SESSION_SETTINGS_EVENT,
  COMPOSER_FOCUS_EVENT,
  OPEN_SESSION_SETTINGS_EVENT,
  pendingSessionSettings,
  type SessionSettingsFocus,
  sessionSettingsFocusFromEvent,
  sessionSettingsEventMatches,
} from "./sessionSettingsOpen";

import { ConfigRevertBack } from "./ConfigRevertBack";
import { PendingPanel, DesktopRegionShortcut, TOOLBAR_ICON_BTN, AttachmentPreviews } from "./PendingPanel";
export { PendingPanel } from "./PendingPanel";

const DesktopComposerToolbar = lazy(async () => {
  const module = await import("./desktop/DesktopComposerToolbar");
  return { default: module.DesktopComposerToolbar };
});

const DesktopApprovalCommands = lazy(async () => {
  const module = await import("./desktop/DesktopApprovalCommands");
  return { default: module.DesktopApprovalCommands };
});
const DesktopApprovalKeycap = lazy(async () => {
  const module = await import("./desktop/DesktopApprovalCommands");
  return { default: module.DesktopApprovalKeycap };
});
/** The numbered `␣1`…`␣9` slots an approval overlay draws on Desktop. */
const DESKTOP_APPROVAL_SLOTS = {
  commands: (
    choices: { label: string; run: () => void }[],
    available: boolean,
  ): React.JSX.Element => (
    <Suspense fallback={null}>
      <DesktopApprovalCommands choices={choices} available={available} />
    </Suspense>
  ),
  keycap: (index: number, available: boolean): React.JSX.Element => (
    <Suspense fallback={null}>
      <DesktopApprovalKeycap index={index} available={available} />
    </Suspense>
  ),
};

const EMPTY_CONFIG_OPTIONS: ConfigOption[] = [];
const EMPTY_QUEUED_MESSAGES: QueuedMessage[] = [];

// Unfilled Send with a font-scaled glyph and a stable minimum touch target.
const MOBILE_SEND_BTN = {
  ...TOOLBAR_ICON_BTN,
  width: "2.5rem",
  height: "2.5rem",
  minWidth: 40,
  minHeight: 40,
  position: "relative",
  p: 0,
  "& .MuiSvgIcon-root": { fontSize: "1.5rem" },
} as const;
// Why an agent-backed toolbar action is greyed out while the session boots. The
// button keeps its slot (no reflow on the ready edge); this explains the state
// instead of leaving a silently dead control.
const AGENT_BOOTING_HINT = "Available once the agent is ready";
const TOOLBAR_MIN_H = {
  minHeight: 34,
  "@media (pointer: coarse)": { minHeight: 40 },
} as const;

// MUI's Button start-icon selector assigns a fixed px size with more
// specificity than an SvgIcon's own sx prop. Own the glyph size at the button
// primitive so both image actions follow the root rem scale truthfully.
const INLINE_IMAGE_ACTION_BUTTON_SX = {
  textTransform: "none",
  fontWeight: 600,
  gap: 0.5,
  px: 1.75,
  py: 1,
  borderRadius: 0,
  "& .MuiButton-startIcon > *": { fontSize: "1.125rem" },
} as const;

// Compact "K/M" token count for the context tooltip (48436 → "48K", 1_000_000 → "1M").
function fmtTokens(n: number): string {
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(n >= 9_950_000 ? 0 : 1)}M`;
  }
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

// Keep the reading optically balanced for one, two, and three digits. The text
// lives in the same 26-unit SVG coordinate space as the gauge, so it scales
// proportionally if the toolbar icon size changes instead of relying on a
// viewport/root-font-dependent CSS size.
function CompactReading(
  { value, muted = false }: { value: number; muted?: boolean },
): React.JSX.Element {
  const fontSize = value < 10 ? 10.5 : value < 100 ? 9.5 : 8.25;
  return (
    <Box
      component="svg"
      viewBox="0 0 26 26"
      aria-hidden="true"
      sx={{
        width: "100%",
        height: "100%",
        color: muted ? "text.disabled" : "text.secondary",
      }}
    >
      <text
        x="13"
        y="13.25"
        textAnchor="middle"
        dominantBaseline="central"
        fill="currentColor"
        fontFamily="system-ui, -apple-system, BlinkMacSystemFont, sans-serif"
        fontSize={fontSize}
        fontWeight="600"
        letterSpacing={value === 100 ? -0.2 : 0}
        style={{ fontVariantNumeric: "tabular-nums" }}
      >
        {value}
      </text>
    </Box>
  );
}

// A thin ring has less visual mass than a filled toolbar glyph, so give it a
// small optical-size correction while keeping it rem-based. It still follows
// the same global font/icon scaling as its neighbours.
const COMPACT_GAUGE_SIZE = "min(1.75rem, 32px)";

// Context-window fullness (agent-reported over ACP `usage_update`, used/size
// tokens) drawn as a Zed-style ring around an integer 0–100 reading — so ONE
// button both shows exactly how full the window is and compacts it. Colour still
// warns at a glance (amber ≥70%, red ≥90% — the auto-compaction zone), while the
// token counts live in the Compact tooltip. Before the agent reports a size it's
// just the bare Compress icon.
export function CompactIcon(
  { used, size, active }: { used: number; size: number; active: boolean },
): React.JSX.Element {
  const hasSize = size > 0;
  const pct = hasSize
    ? (used > 0
      ? Math.max(1, Math.round(Math.min(100, (used / size) * 100)))
      : 0)
    : 0;
  // Compaction running right now → an indeterminate terracotta (Claude accent,
  // matching the transcript's CompactingWidget) spinner around a dimmed reading, in
  // the same icon footprint so the toolbar doesn't shift. The button is disabled
  // in this state, so the ring reads as "working", not an affordance.
  if (active) {
    return (
      <Box
        sx={{
          position: "relative",
          width: COMPACT_GAUGE_SIZE,
          height: COMPACT_GAUGE_SIZE,
          flexShrink: 0,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <CircularProgress
          size={COMPACT_GAUGE_SIZE}
          thickness={3}
          sx={{ color: "#D97757", position: "absolute", top: 0, left: 0 }}
        />
        {hasSize
          ? <CompactReading value={pct} muted />
          : <Compress sx={{ fontSize: "1.05rem", color: "text.disabled" }} />}
      </Box>
    );
  }
  if (!hasSize) return <Compress fontSize="small" />;
  const color = pct >= 90
    ? "error.main"
    : pct >= 70
    ? "warning.main"
    : "success.main";
  return (
    <Box
      sx={{
        position: "relative",
        width: COMPACT_GAUGE_SIZE,
        height: COMPACT_GAUGE_SIZE,
        flexShrink: 0,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Box
        component="svg"
        viewBox="0 0 26 26"
        aria-hidden="true"
        sx={(theme) => ({
          position: "absolute",
          inset: 0,
          width: "100%",
          height: "100%",
          color,
          overflow: "visible",
          "& .compact-gauge-track": {
            stroke: alpha(theme.palette.text.secondary, 0.38),
          },
        })}
      >
        <circle
          className="compact-gauge-track"
          cx="13"
          cy="13"
          r="11"
          fill="none"
          strokeWidth="2"
        />
        {pct > 0 && (
          <circle
            cx="13"
            cy="13"
            r="11"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap={pct < 100 ? "round" : undefined}
            pathLength="100"
            strokeDasharray={pct < 100 ? `${pct} ${100 - pct}` : undefined}
            transform="rotate(-90 13 13)"
          />
        )}
      </Box>
      <CompactReading value={pct} />
    </Box>
  );
}

// The Compact tooltip — names the action and, when known, the context fullness it
// acts on ("Compact · context 79% · 780K / 1M tokens").
export function compactTooltip(used: number, size: number): string {
  if (!(size > 0)) return "Compact conversation";
  const pct = used > 0
    ? Math.max(1, Math.round(Math.min(100, (used / size) * 100)))
    : 0;
  return `Compact conversation · context ${pct}% · ${fmtTokens(used)} / ${
    fmtTokens(size)
  } tokens`;
}

function SessionActionConfirmDialog({
  action,
  provider,
  activeTurn,
  disableRestoreFocus = false,
  onClose,
  onConfirm,
}: {
  action: SessionAction | null;
  provider: string;
  activeTurn: boolean;
  disableRestoreFocus?: boolean;
  onClose: () => void;
  onConfirm: () => Promise<void>;
}): React.JSX.Element {
  void disableRestoreFocus;
  return (
    <ConfirmSheet
      open={action !== null}
      onClose={onClose}
      title={action !== null ? `${action.label}?` : undefined}
      actions={action === null ? undefined : (
        <>
          <Button
            color="inherit"
            onClick={onClose}
            sx={{ textTransform: "none" }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <NetworkButton
            variant="contained"
            color={action.destructive ? "error" : "primary"}
            networkAction={onConfirm}
            sx={{ textTransform: "none" }}
          >
            {action.destructive ? "Clear" : "Compact"}
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </NetworkButton>
        </>
      )}
    >
      {action !== null && (
        <>
          <DialogContentText>{action.detail}</DialogContentText>
          <ConfirmConsequence
            tone={action.destructive ? "error" : "primary"}
            irreversible={action.destructive}
          >
            {action.kind === "slash" && action.command !== undefined
              ? (
                <>
                  Sends{" "}
                  <Box
                    component="code"
                    sx={{
                      fontFamily: "ui-monospace, monospace",
                      px: 0.5,
                      py: 0.125,
                      borderRadius: 0.75,
                      bgcolor: "action.hover",
                    }}
                  >
                    {action.command}
                  </Box>{" "}
                  to {provider || "the agent"}
                  {activeTurn ? " (queued — the agent is mid-turn)" : ""}.
                </>
              )
              : `Resets ${provider || "the agent"} to a fresh context now${
                activeTurn ? " (ends the current turn)" : ""
              }.`}
          </ConfirmConsequence>
        </>
      )}
    </ConfirmSheet>
  );
}

export function ComposerWorkspace({
  sessionId,
  status,
  variant = "overlay",
  surface = "mobile",
  autoFocus = false,
  onSubmitted,
}: ComposerWorkspaceProps): React.JSX.Element {
  /// "overlay" (default): the composer floats over the transcript at the bottom
  /// (single-column / mobile). "column": the desktop two-column layout — the
  /// composer is a full-height left column (queued + drafts scroll at the top,
  /// the editor card fills the rest), so it does NOT float and the
  /// compact↔expand toggle + drag-resize handle are dropped (the column height
  /// IS the editor size). See desktopLayout.ts.
  // `variant` is intentionally constrained by the product shell wrappers:
  // Mobile always requests overlay; only Desktop can request a column.
  // Two-column (desktop split) mode — gates every overlay-specific affordance
  // (floating-stack reservation lives in App, expand toggle, resize
  // handle) off and turns the root into a fill-height flex column instead.
  const column = variant === "column";
  const desktop = surface === "desktop";
  // Rollback switch for the Calls panel (docs/background-tasks.md §6.3).
  const callsDockLegacy = useCallsDockLegacy();
  const touchInput = useTouchComposer();
  const keyboardOpen = useKeyboardOpen();
  const [mobileInputResetBlocked, setMobileInputResetBlocked] = useState(false);
  const [mobileKeyboardDismissed, setMobileKeyboardDismissed] = useState(false);
  const mobileKeyboardPresentationOpen = shouldPresentMobileKeyboardSurface(
    keyboardOpen,
    mobileInputResetBlocked,
    mobileKeyboardDismissed,
  );
  const preparing = status === "starting";
  // The context cue strip hides while the mobile keyboard owns the screen, so
  // the placeholder repeats that the next message starts a new agent.
  const linkedContext = useAnnouncedSessionContext(sessionId);
  const messagePlaceholder = linkedContext?.next_agent &&
      linkedContext.revision !== linkedContext.applied_revision
    ? "Message the new agent…"
    : "Message the agent…";
  const queue = useStoreSelector((snapshot) =>
    snapshot.queues.get(sessionId) ?? EMPTY_QUEUED_MESSAGES
  );
  const editorRef = useRef<ComposerEditorHandle>(null);
  useEffect(() => {
    const focus = (event: Event): void => {
      if (!sessionSettingsEventMatches(event, sessionId)) return;
      // Let the closing settings surface release focus first.
      globalThis.setTimeout(() => editorRef.current?.focusEnd(), 0);
    };
    document.addEventListener(COMPOSER_FOCUS_EVENT, focus);
    return () => document.removeEventListener(COMPOSER_FOCUS_EVENT, focus);
  }, [sessionId]);
  const {
    text,
    setText,
    attachments,
    setAttachments,
    initialText: initialDraftText,
    sendable,
    addFiles,
    pasteClipboardImages,
    removeAttachment,
    clear: clearComposer,
    submit,
    submitTracked,
    currentTurnTracked,
    jumpToFrontTracked: jumpCurrentPromptToFrontTracked,
    saveAsDraftTracked,
    scheduleNew,
  } = useComposerDraftController(sessionId, editorRef, {
    // Desktop CodeMirror owns its document. Keeping its hot path in refs avoids
    // rerendering the entire Prompt pane for every IME composition update;
    // touch-native editors still require the live React value.
    mirrorTextInReact: surface !== "desktop",
  });
  const fileDrop = useComposerFileDrop(
    !touchInput,
    (files): void => addFiles(files, { preserveFocus: true }),
  );
  // Clearing is independent from sending. A still-uploading attachment cannot
  // be sent yet, but it is still user-owned staged content and must remain
  // removable from the utility rail.
  const clearable = text.trim().length > 0 || attachments.length > 0;
  const [deliveryAnchor, setDeliveryAnchor] = useState<HTMLElement | null>(
    null,
  );
  const deliverySource = useRef<
    {
      session: string;
      text: string;
      attachments: Attachment[];
      selection: ComposerEditorSelection | undefined;
    } | null
  >(null);
  const openDeliveryPicker = useCallback((): boolean => {
    if (!sendable) return false;
    const anchor = queueBtnRef.current ?? document.activeElement;
    if (!(anchor instanceof HTMLElement)) return false;
    deliverySource.current = {
      session: sessionId,
      text: editorRef.current?.getValue() ?? text,
      attachments,
      selection: editorRef.current?.getSelection(),
    };
    setDeliveryAnchor(anchor);
    if (!desktop) dismissMobileSoftwareKeyboard();
    return true;
  }, [attachments, desktop, sendable, sessionId, text]);
  const closeDeliveryPicker = (): void => {
    const source = deliverySource.current;
    flushSync(() => {
      setDeliveryAnchor(null);
    });
    if (source?.session !== sessionId) return;
    resumeMobileKeyboardSurface();
    if (source.selection) editorRef.current?.focusSelection(source.selection);
    else editorRef.current?.focus();
  };
  const deliverySourceCurrent = (): boolean => {
    const source = deliverySource.current;
    return source?.session === sessionId &&
      source.attachments === attachments &&
      source.text === (editorRef.current?.getValue() ?? text);
  };
  useEffect(() => {
    setDeliveryAnchor(null);
    deliverySource.current = null;
  }, [sessionId]);
  // Sending during startup is QUEUEING, not an error: the session is not
  // dispatchable while it boots, so store.submitPrompt routes the prompt to the
  // daemon queue and the daemon's own status-change drain fires it the instant
  // the agent reports Running. Blocking it here made the "you can start typing
  // while this session prepares…" placeholder a lie — ⌘↵ silently did nothing.
  const submitAndNotify = useCallback((): boolean => {
    if (status === "busy" || queue.length > 0) return openDeliveryPicker();
    const submitted = submit();
    if (submitted) {
      if (!desktop) dismissMobileSoftwareKeyboard();
      onSubmitted?.();
    }
    return submitted;
  }, [desktop, onSubmitted, openDeliveryPicker, queue.length, status, submit]);
  const submitFeedback = useNetworkActionState();
  // Mobile-only fullscreen compose: the ↗ opens a near-full-screen sheet (the
  // first-class long-form / future-markdown editor). Desktop keeps the Zed-style
  // inline expand instead (composeFs is never set true there).
  const [composeFs, setComposeFs] = useState(false);
  const dismissAfterMobileDelivery = useCallback((): void => {
    if (desktop) return;

    // Delivery ends this keyboard interaction even when WKWebView reports its
    // resized viewport for a few more frames. Keep the composer collapsed until
    // the user explicitly touches the editor again; a lagging keyboardOpen must
    // not resurrect the large focused surface after the prompt has been sent.
    noteMobileKeyboardDismissed();
    setMobileKeyboardDismissed(true);
    // A touch click may have moved activeElement from the editor to its toolbar
    // button by the time the authoritative acknowledgement arrives. Clear both
    // possible owners. Modal/Popover teardown can restore the old Composer focus
    // on the following paint, so repeat once after that restoration opportunity.
    dismissMobileSoftwareKeyboard();
    releaseMobileComposerFocus();
    globalThis.requestAnimationFrame(() => {
      dismissMobileSoftwareKeyboard();
      releaseMobileComposerFocus();
    });
  }, [desktop]);
  // Queue/Draft row taps are a fresh writing gesture. Delivery and hide-keyboard
  // latch the compact surface so a lagging visualViewport cannot resurrect the
  // empty primary card; that latch must not also freeze the pending editor in
  // its compact "N Drafts Editing" card after iOS has already raised the keyboard.
  const resumeMobileKeyboardSurface = useCallback((): void => {
    clearMobileKeyboardDismissed();
    setMobileKeyboardDismissed(false);
  }, []);
  const submitWithFeedback = useCallback((onSucceeded?: () => void): void => {
    void (async () => {
      let submitted = false;
      const succeeded = await submitFeedback.run(() => {
        const confirmation = submitTracked();
        if (confirmation === null) return Promise.resolve();
        submitted = true;
        return confirmation;
      });
      if (submitted && succeeded) {
        dismissAfterMobileDelivery();
        // Explore/Page conditionally mounts this Composer. Keep it mounted
        // through the authoritative acknowledgement so the first tap does not
        // combine delivery with a Page Dock layout teardown.
        onSubmitted?.();
        onSucceeded?.();
      }
    })();
  }, [
    dismissAfterMobileDelivery,
    onSubmitted,
    submitFeedback,
    submitTracked,
  ]);
  const mobileActionClickGuard = useRetargetedTouchClickGuard<HTMLDivElement>();
  const sendTap = useReliableTouchTap<HTMLButtonElement>(
    () => requestDelivery(),
    mobileActionClickGuard.arm,
  );
  const fileInputRef = useRef<HTMLInputElement>(null);
  const draftList = useStoreSelector((snapshot) =>
    snapshot.drafts.get(sessionId) ?? EMPTY_QUEUED_MESSAGES
  );
  const sessionState = useStoreSelector(
    (snapshot) => composerSessionSlice(snapshot.sessions, sessionId),
    sameComposerSessionSlice,
  );
  const supportsCurrentTurn = useStoreSelector((snapshot) =>
    snapshot.sessions.find((session) => session.id === sessionId)
      ?.supports_current_turn === true
  );
  const addToCurrentTurn = (
    target?: DeliveryTarget,
    nextCheckpoint = false,
  ): void => {
    let submitted = false;
    void submitFeedback.run(() => {
      const confirmation = currentTurnTracked(target, nextCheckpoint);
      if (confirmation === null) return Promise.resolve();
      submitted = true;
      return confirmation;
    })
      .then((succeeded) => {
        if (submitted && succeeded) {
          dismissAfterMobileDelivery();
          setComposeFs(false);
          onSubmitted?.();
        }
      });
  };
  const timelineState = useStoreSelector(
    (snapshot) => composerTimelineSlice(snapshot.timelines.get(sessionId)),
    sameComposerTimelineSlice,
  );
  // `queues`/`drafts` already merge the server rows with this device's optimistic
  // (sending/failed) rows via the queue sync client (commitQueue) — server rows
  // first, optimistic rebased after, reconciled out the instant their cmid lands.
  // The agent's current plan, pinned above the queue as a collapsible dock so
  // task progress stays in view without scrolling the transcript. null = no plan.
  const plan = timelineState.plan;
  // The latest unresolved tool-permission request (cheap single pass). When set,
  // the sticky PermissionOverlay takes the floating slot INSTEAD of the
  // turn-status pill — the two share the slot + material but never show at once.
  const pendingPermission = timelineState.pendingPermission;
  // Agent calls the session's policy refused, waiting for this person's
  // answer. It ranks below a tool permission and above the turn status.
  const callApproval = useCallApproval(sessionId);
  // Desktop answers both in the Conversation pane's dock; this stack keeps
  // them only where no dock is on screen (Mobile, a folded Conversation).
  const approvalDocked = useApprovalDockOwns(sessionId);
  // Manual dismiss: keyed on the plan's step list so it stays gone as the agent
  // updates statuses, but a genuinely new plan (different steps) reappears.
  const [dismissedPlanKey, setDismissedPlanKey] = useState<string | null>(null);
  const planKey = plan?.key ?? null;
  const dismissPlan = useCallback((): void => {
    if (planKey !== null) setDismissedPlanKey(planKey);
  }, [planKey]);
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
  const theme = useTheme();
  // Touch tier collapses the agent config into a single Tune button — tapping
  // it opens a BottomSheet with the session info + every config option in one
  // place. Inspired by ChatGPT / DeepSeek / Gemini: chips wrap awkwardly on
  // iPad portrait (820px) and are completely unreadable on a 390px iPhone, so
  // the sheet pattern wins on every sub-desktop viewport. Desktop keeps the
  // inline chip row — there's room.
  const compact = useMediaQuery(theme.breakpoints.down("lg"));
  const mobileToolbarIds = useComposerToolbar();
  const [mobileToolbarSettingsOpen, setMobileToolbarSettingsOpen] = useState(
    false,
  );
  // A Queue/Draft edit is itself a complete composer. Mobile must expose one
  // writing focus at a time: leaving the new-message composer underneath the
  // active row editor creates two large, nearly identical cards above the
  // keyboard and makes the lower one look actionable even though focus belongs
  // to the row. Keep each panel's local edit ownership, but project it here so
  // the ordinary composer can yield its visual slot until editing completes.
  const [mobileQueuedEditing, setMobileQueuedEditing] = useState(false);
  const [mobileDraftEditing, setMobileDraftEditing] = useState(false);
  const mobilePendingEditing = mobileQueuedEditing || mobileDraftEditing;
  const handedOff = useHandedOff(sessionId);
  const onMobileQueuedEditingChange = useCallback((editing: boolean): void => {
    setMobileQueuedEditing(editing);
  }, []);
  const onMobileDraftEditingChange = useCallback((editing: boolean): void => {
    setMobileDraftEditing(editing);
  }, []);
  // Inline-image selection popover. Tapping an inline image opens a small popover
  // (Preview / Delete) anchored to its <img>, ringed while open. The image widget
  // lives outside React, so it calls a module-level tap handler we register here.
  const [imgSel, setImgSel] = useState<
    { id: string; el: HTMLElement; x: number; y: number } | null
  >(null);
  const closeImgSel = useCallback((): void => {
    setImgSel((cur) => {
      cur?.el.classList.remove("cm-inline-image-selected");
      return null;
    });
  }, []);
  const previewSelectedImage = useCallback((): void => {
    // Resolve from the INLINE-IMAGE REGISTRY (all surfaces register there),
    // not the local `attachments` — otherwise Preview no-ops in the expanded
    // editor whose image lives in editAttachments.
    const id = imgSel?.id;
    const att = (id ? getInlineAttachment(id) : undefined) ??
      attachments.find((attachment) => attachment.id === id);
    // Preview leaves writing for the lightbox, so collapse two-track chrome
    // and drop the software keyboard instead of leaving ∧∨✓ over the image.
    noteMobileKeyboardDismissed();
    setMobileKeyboardDismissed(true);
    dismissMobileSoftwareKeyboard();
    releaseMobileComposerFocus();
    if (att) openLightbox([att], 0);
    closeImgSel();
  }, [attachments, closeImgSel, imgSel]);
  const deleteSelectedImage = useCallback((): void => {
    if (imgSel) {
      editorRef.current?.deleteImage(imgSel.id);
      setAttachments((previous) =>
        previous.filter((attachment) => attachment.id !== imgSel.id)
      );
    }
    closeImgSel();
  }, [closeImgSel, imgSel, setAttachments]);
  // Commit stationary image actions on pointerup while retaining the native
  // click as an independent WebKit fallback. Preventing the TOUCH pointerdown
  // suppresses that fallback; prevent only its compatibility mousedown below
  // so the editor keeps focus without making pointerup the sole activation path.
  const imagePreviewTap = useReliableTouchTap<HTMLButtonElement>(
    previewSelectedImage,
  );
  const imageDeleteTap = useReliableTouchTap<HTMLButtonElement>(
    deleteSelectedImage,
  );
  useEffect(() => {
    setImageTapHandler((id, el, x, y) => {
      setImgSel((prev) => {
        prev?.el.classList.remove("cm-inline-image-selected");
        el.classList.add("cm-inline-image-selected");
        return { id, el, x, y };
      });
    });
    return (): void => setImageTapHandler(null);
  }, []);
  useEffect(() => {
    if (imgSel === null) return undefined;
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape" || isImeKeyEvent(event)) return;
      event.preventDefault();
      event.stopPropagation();
      closeImgSel();
    };
    globalThis.addEventListener("keydown", closeOnEscape, true);
    return (): void =>
      globalThis.removeEventListener("keydown", closeOnEscape, true);
  }, [closeImgSel, imgSel]);
  // The expand tap transfers focus synchronously below. Never refocus from an
  // effect: a later programmatic focus replaces iOS's user-armed text
  // interaction and suppresses long-press Paste/Select.
  // Stopping a running turn is confirmed through a modal (Enter confirms, Esc
  // dismisses) — clicking Stop or pressing Esc in the editor opens it, rather
  // than cancelling on a single stray click/keypress.
  const [cancelOpen, setCancelOpen] = useState(false);
  // Long-press and ordinary busy Send share the same picker.
  const [holding, setHolding] = useState(false);
  const [clearComposerAnchor, setClearComposerAnchor] = useState<
    HTMLElement | null
  >(null);
  useEffect(() => {
    if (clearComposerAnchor === null) return undefined;
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape" || isImeKeyEvent(event)) return;
      event.preventDefault();
      event.stopPropagation();
      setClearComposerAnchor(null);
    };
    globalThis.addEventListener("keydown", closeOnEscape, true);
    return (): void =>
      globalThis.removeEventListener("keydown", closeOnEscape, true);
  }, [clearComposerAnchor]);
  // Send owns the picker anchor for pointer and keyboard activation alike.
  const queueBtnRef = useRef<HTMLButtonElement | null>(null);
  const mobileActionsRef = useRef<HTMLDivElement | null>(null);
  const [mobileActionEdges, setMobileActionEdges] = useState({
    left: false,
    right: false,
  });
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
  const otherSessions = sessionState.destinations;
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
  // The agent's predicted next prompt. Offered only while nothing runs and the
  // composer is empty; taking it fills the composer and never sends.
  const predictedPrompt = usePromptSuggestion(sessionId);
  const offeredPrompt = predictedPrompt !== null && !busy && !preparing &&
      attachments.length === 0
    ? predictedPrompt
    : null;
  const mobilePromptSuggestion = !desktop && !composeFs && text === ""
    ? offeredPrompt
    : null;
  function requestDelivery(onSucceeded?: () => void): void {
    if (status === "busy" || queue.length > 0) openDeliveryPicker();
    else submitWithFeedback(onSucceeded);
  }
  const starting = status === "starting";
  // Without a live server a send is queued until Cowboy is reachable, so the
  // button names that outcome instead of promising an immediate send.
  const serverConnected = useConnected();
  const currentTurnAvailable = serverConnected && status === "busy" &&
    supportsCurrentTurn;
  // Warm the delivery options as a turn starts, so the busy Send opens on
  // the session's real policy and checkpoint support without a wait.
  useEffect(() => {
    if (currentTurnAvailable) prefetchDeliveryOptions(sessionId);
  }, [currentTurnAvailable, sessionId]);
  // A compaction is running right now: the turn is busy AND the live tail is
  // Claude Code's "Compacting..." notice (covers both a hand-fired /compact and
  // the agent's own auto-compaction). Drives the Compact button's disabled +
  // spinner state so a second /compact can't be fired mid-run.
  const compacting = busy && timelineState.compactingTail;
  // Pause controls automatic drain independently of this message's delivery.
  const paused = sessionState.paused;
  // Interrupted is a dead/resumable state too (a turn cut off by a daemon
  // restart) — the composer treats it like exited/crashed: "send to resume".
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
  // Session startup is a SESSION-level state, so it is presented on session-level
  // surfaces (the navbar StatusDot, the composer's top-edge progress line, the
  // transcript empty state) — NOT by restructuring this toolbar. Every action
  // keeps its slot while the agent boots and only loses `enabled`, so the row
  // never reflows on the ready edge and the affordances stay discoverable.
  // Blocked here are exactly the actions that need a prepared workspace;
  // attaching, clearing the editor, saving a draft and sending (→ queue) are
  // local/daemon work and stay usable — the placeholder promises the user can
  // start writing. A stopped session is NOT blocked: Compact goes down the
  // prompt path, which wakes it, and Clear resets a hibernated session without
  // starting a worker, so neither needs a throwaway message first.
  const agentActionsDisabled = preparing;
  // Delivery choices require an active turn or a held queue. Startup queues
  // normally until the agent reports Running.
  const forceable = !preparing && (busy || starting || paused);
  // "Working" is EXACTLY the ACP turn being in flight (`busy` = a session/prompt
  // request that hasn't returned a stop_reason) — the same predicate Zed uses
  // (`ThreadStatus::Generating == running_turn.is_some()`). We deliberately do NOT
  // re-derive it from tool-call status in the transcript: claude-agent-acp emits no
  // `in_progress`, and its terminal `completed`/`failed` updates are unreliable
  // (measured: dangling tool calls survive 1–27 later turns — mostly lost
  // terminals, not live work), so a tool-in-flight heuristic showed false "working"
  // on long-finished sessions. A tool only ever runs while the prompt is in flight,
  // so `busy` already covers it; nothing reliable exists to add between turns.
  const turnWorking = busy;
  // A dead session is still sendable: sending resumes it (the daemon revives
  // the agent via session/load — see supervisor.rs). Matches Zed, where a
  // thread is never permanently unusable just because its agent process ended.
  // An attachment-only prompt (e.g. just a pasted screenshot) is also sendable.
  // Slash skills + `@` file references are handled inside the editor now, via
  // CodeMirror autocomplete (see ComposerEditor + composerCompletions): no more
  // Popper pickers or caret/regex bookkeeping here. The editor reads the
  // agent-advertised `/` commands through a thunk; `@` files come from the
  // daemon's `/api/sessions/{id}/files` search.
  const availableCommands = timelineState.availableCommands;

  // Session-lifecycle one-tap actions (Compact / Clear). Resolved per agent from
  // the agent's advertised command list + a provider default (see
  // agentCommands.ts); `null` when this agent offers no equivalent, so the button
  // hides rather than sending a command the agent can't parse. A tap opens a
  // confirm dialog first (Clear drops history — not undoable); confirming sends
  // the slash-command down the SAME prompt path as a typed message, so it queues
  // if the agent is mid-turn, exactly like sending "/compact" by hand.
  const provider = sessionState.provider;
  const providerVersion = sessionState.providerVersion;
  const providerDigest = sessionState.providerDigest;
  const compactAction = useMemo(
    () =>
      resolveSessionAction(
        "compact",
        provider,
        availableCommands,
        providerVersion,
        providerDigest,
      ),
    [provider, providerVersion, providerDigest, availableCommands],
  );
  const clearAction = useMemo(
    () =>
      resolveSessionAction(
        "clear",
        provider,
        availableCommands,
        providerVersion,
        providerDigest,
      ),
    [provider, providerVersion, providerDigest, availableCommands],
  );
  const [cmdConfirm, setCmdConfirm] = useState<SessionAction | null>(null);
  const contextClearedRef = useRef({
    sessionId,
    seq: timelineState.contextClearedSeq,
  });
  useEffect(() => {
    const previous = contextClearedRef.current;
    contextClearedRef.current = {
      sessionId,
      seq: timelineState.contextClearedSeq,
    };
    if (previous.sessionId !== sessionId) {
      setMobileInputResetBlocked(false);
      return;
    }
    if (timelineState.contextClearedSeq <= previous.seq) return;
    if (touchInput) {
      setMobileInputResetBlocked(true);
      dismissMobileSoftwareKeyboard();
      releaseMobileComposerFocus();
    }
  }, [sessionId, timelineState.contextClearedSeq, touchInput]);
  const compactContext = useCompactionContext({
    sessionId,
    status,
    serverUsed: sessionState.contextUsed,
    serverSize: sessionState.contextSize,
    completionSeq: timelineState.completionSeq,
  });
  function runSessionAction(a: SessionAction): Promise<void> {
    haptic();
    if (a.kind === "reset") {
      // Clear: a cowboy session reset (fresh agent context), not a prompt.
      return resetSession(sessionId);
    } else if (a.command !== undefined) {
      // Compact: send the agent's slash-command down the normal prompt path.
      compactContext.beginRefresh();
      return submitPrompt(sessionId, a.command, []).catch((error: unknown) => {
        compactContext.cancelRefresh();
        throw error;
      });
    }
    return Promise.resolve();
  }
  async function confirmSessionAction(): Promise<void> {
    if (cmdConfirm === null) return;
    const action = cmdConfirm;
    // A context reset is a hard end to the current input interaction. Close the
    // Dialog before the request, then clear both the current focus owner and the
    // focus MUI may restore while the closing transition commits. Without the
    // post-close pass WebKit can resurrect a stale textarea first responder and
    // combine it with a lagging visualViewport keyboard measurement, leaving a
    // tall floating composer over a keyboard-free transcript.
    setCmdConfirm(null);
    if (touchInput && action.kind === "reset") {
      setMobileInputResetBlocked(true);
      setComposeFs(false);
      setClearComposerAnchor(null);
      setImgSel(null);
      dismissMobileSoftwareKeyboard();
      releaseMobileComposerFocus();
      globalThis.requestAnimationFrame(() => releaseMobileComposerFocus());
    }
    await runSessionAction(action);
  }
  useConfirmEnter(cmdConfirm !== null, () => {
    void confirmSessionAction();
  });

  // Vim is opt-in and desktop-only — ComposerEditor gates the actual
  // `@replit/codemirror-vim` load on a precise-pointer device, so touch never
  // pays for it. The reactive setting is flipped by the Settings toggle.
  const vim = useVimSetting();
  // Expand toggle (desktop only — gated where rendered). Persisted per device.
  const expanded = useComposerExpanded();
  // That preference can be shared by PWA storage across iPad/Desktop surface
  // classifications. Mobile owns a separate fullscreen sheet, so its ordinary
  // composer must never inherit Desktop's 48vh inline canvas.
  const inlineExpanded = shouldExpandInlineComposer(
    desktop ? "desktop" : "mobile",
    expanded,
  );
  const composerHeight = useComposerHeight();
  // Drag-to-resize the editor (desktop), VSCode-terminal style: a top-edge handle
  // grows/shrinks the editor; dragging below RESIZE_MIN auto-collapses to the
  // compact auto-grow editor, dragging back up auto-expands. `editorAreaRef`
  // measures the live editor height to seed the drag from the compact state.
  const editorAreaRef = useRef<HTMLDivElement>(null);
  const RESIZE_MIN = 96;
  const onResizeStart = useCallback((e: ReactPointerEvent): void => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startY = e.clientY;
    const startH = expanded
      ? (composerHeight > 0
        ? composerHeight
        : Math.round(globalThis.innerHeight * 0.48))
      : (editorAreaRef.current?.clientHeight ?? RESIZE_MIN);
    const maxH = Math.round(globalThis.innerHeight * 0.82);
    const doc = globalThis.document;
    const move = (ev: PointerEvent): void => {
      const next = startH + (startY - ev.clientY); // drag up → taller
      if (next < RESIZE_MIN) {
        setComposerExpanded(false); // snap to the compact auto-grow editor
      } else {
        setComposerExpanded(true);
        setComposerHeight(Math.min(next, maxH));
      }
    };
    const end = (): void => {
      globalThis.removeEventListener("pointermove", move);
      globalThis.removeEventListener("pointerup", end);
      doc.body.style.cursor = "";
      doc.body.style.userSelect = "";
    };
    globalThis.addEventListener("pointermove", move);
    globalThis.addEventListener("pointerup", end);
    doc.body.style.cursor = "ns-resize";
    doc.body.style.userSelect = "none";
  }, [expanded, composerHeight]);
  // Touch and Desktop share CM6 document semantics. Touch keeps Vim disabled,
  // while preserving inline image tokens across compact/fullscreen handoff.
  useLayoutEffect(() => {
    if (!touchInput) return undefined;
    const track = mobileActionsRef.current;
    if (!track) return undefined;
    const measure = (): void => {
      const maxScroll = Math.max(0, track.scrollWidth - track.clientWidth);
      const next = {
        left: track.scrollLeft > 2,
        right: track.scrollLeft < maxScroll - 2,
      };
      setMobileActionEdges((current) =>
        current.left === next.left && current.right === next.right
          ? current
          : next
      );
    };
    measure();
    track.addEventListener("scroll", measure, { passive: true });
    const resizeObserver = new ResizeObserver(measure);
    resizeObserver.observe(track);
    const mutationObserver = new MutationObserver(measure);
    mutationObserver.observe(track, { childList: true, subtree: true });
    return (): void => {
      track.removeEventListener("scroll", measure);
      resizeObserver.disconnect();
      mutationObserver.disconnect();
    };
  }, [touchInput]);
  // Editing ownership and visual expansion are deliberately separate. Queue or
  // Draft may still own the buffer while WebKit is dismissing (or declined to
  // open) the keyboard; that must not grant a mobile surface fill-height.
  const mobileComposerKeyboardWasOpenRef = useRef(false);
  useLayoutEffect(() => {
    if (!touchInput) {
      mobileComposerKeyboardWasOpenRef.current = false;
      return undefined;
    }
    if (keyboardOpen) {
      const opening = !mobileComposerKeyboardWasOpenRef.current;
      mobileComposerKeyboardWasOpenRef.current = true;
      if (!opening) return undefined;
      const reveal = (): void => {
        if (editorRef.current?.hasFocus()) {
          editorRef.current.revealSelection();
        }
      };
      // Keyboard height, composer max-height, and inline-image layout all
      // settle after the first frame. One rAF under-scrolls the caret behind
      // the keyboard when a tall image is still measuring.
      let laterFrame = 0;
      const frame = globalThis.requestAnimationFrame(() => {
        reveal();
        laterFrame = globalThis.requestAnimationFrame(reveal);
      });
      const timer = globalThis.setTimeout(reveal, 260);
      return (): void => {
        globalThis.cancelAnimationFrame(frame);
        if (laterFrame !== 0) globalThis.cancelAnimationFrame(laterFrame);
        globalThis.clearTimeout(timer);
      };
    }
    if (
      !didMobileSoftwareKeyboardClose(
        mobileComposerKeyboardWasOpenRef.current,
        keyboardOpen,
      )
    ) return undefined;

    if (isMobileEditorFocusTransferPending()) {
      // Expand → collapse remounts the editor. visualViewport can report
      // closed while the software keyboard is still up; do not treat that
      // flicker as a user dismiss or we latch the compact chrome over it.
      return undefined;
    }

    mobileComposerKeyboardWasOpenRef.current = false;
    // iOS and third-party keyboards can hide without blurring their surviving
    // textarea/contenteditable. Collapse the compact card's keyboard fill and
    // end that stale session at the visualViewport open→closed boundary.
    noteMobileKeyboardDismissed();
    setMobileKeyboardDismissed(true);
    releaseMobileComposerFocus();
    return undefined;
  }, [keyboardOpen, touchInput]);
  const compactTrayAttachments = attachmentTrayForSurface(
    attachments,
    text,
  );
  const fullscreenTrayAttachments = attachmentTrayForSurface(
    attachments,
    text,
  );

  // --- Long-press → delivery picker -------------------------------------------
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
    // Keep the native textarea as iOS's first responder until the tap/hold is
    // resolved. Blurring it here starts keyboard dismissal and can delay or
    // swallow the trailing click after the Composer reflows.
    e.preventDefault();
    if (!sendable) return;
    // Startup has no active turn; its hold falls through to ordinary enqueue.
    if (!forceable) return;
    lpFired.current = false;
    lpStart.current = { x: e.clientX, y: e.clientY };
    setHolding(true);
    lpTimer.current = globalThis.setTimeout(() => {
      lpTimer.current = undefined;
      lpFired.current = true;
      setHolding(false);
      haptic();
      openDeliveryPicker();
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
    requestDelivery();
  }
  // "Jump to front of queue" (no interrupt): send the composed prompt to the
  // FRONT of the queue so it runs next after the current turn, ahead of the rest
  // of the queue. Only meaningful when there's already a queue to jump ahead of.
  function jumpToFront(): void {
    const confirmation = jumpCurrentPromptToFrontTracked(queue.length);
    if (confirmation === null) return;
    void confirmation.then(() => {
      dismissAfterMobileDelivery();
      setComposeFs(false);
    }).catch(() => undefined);
  }
  useEffect(() => (): void => {
    if (lpTimer.current !== undefined) globalThis.clearTimeout(lpTimer.current);
  }, []);

  // Park the composer's content as a draft (the Draft button) and clear the
  // input. Drafts persist and are activated later from the Drafts panel.
  function saveDraft(): void {
    const confirmation = saveAsDraftTracked();
    if (confirmation === null) return;
    void confirmation.then(dismissAfterMobileDelivery).catch(() => undefined);
  }

  // The schedule picker: `null` = closed; `{id}` present = editing an existing
  // scheduled draft (reschedule/cancel); `{id: undefined}` = scheduling the
  // composer's CURRENT content into a fresh draft.
  const [scheduleTarget, setScheduleTarget] = useState<
    { id: string | undefined; initial: DraftSchedule | null } | null
  >(null);
  function commitSchedule(fireAtMs: number, delivery: Delivery): void {
    if (scheduleTarget?.id !== undefined) {
      void scheduleDraft(sessionId, {
        id: scheduleTarget.id,
        fireAtMs,
        delivery,
      }).catch(() => undefined);
      return;
    }
    // Fresh: schedule the composer's content, then clear the input like saveDraft.
    if (scheduleNew(fireAtMs, delivery)) dismissAfterMobileDelivery();
  }

  return (
    <Box
      data-mobile-composer-workspace={!desktop ? "true" : undefined}
      onPointerDownCapture={(event): void => {
        if (!touchInput || !mobileInputResetBlocked) return;
        const target = event.target instanceof HTMLElement
          ? event.target
          : null;
        if (
          target?.closest(
            "[data-mobile-editor-area], [data-pending-edit-target], [data-mobile-pending-editor]",
          )
        ) setMobileInputResetBlocked(false);
      }}
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
        // Mobile owns vertical rhythm across the whole composer stack. Keep the
        // same small gap below the transcript hairline as between mounted Plan,
        // Pending, and Composer children so the first surface never fuses with
        // the transcript boundary.
        "--mobile-composer-stack-gap": `${mobileComposerStackGap}px`,
        "--mobile-composer-boundary-gap": `${mobileComposerStackGap}px`,
        pt: desktop ? 0 : "var(--mobile-composer-boundary-gap)",
        display: "flex",
        flexDirection: "column",
        alignItems: "stretch",
        rowGap: desktop ? 0 : "var(--mobile-composer-stack-gap)",
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
        // Bottom inset only when the composer is the bottom-most element. With
        // the navbar at the bottom it sits below us and owns the home-indicator
        // inset, so we drop to a plain (tight) gap.
        pb: navbarAtBottom
          ? `${mobileComposerKeyboardGap}px`
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
        ...(!desktop && {
          minHeight: 0,
          maxHeight: "100%",
          // Every visible slot shares one horizontal contract. Explicitly zero
          // the flex minimum so a long pending row, CodeMirror canvas, or
          // container-query child cannot shrink or widen the whole bottom stack.
          "& > *": {
            width: "100%",
            minWidth: 0,
            maxWidth: "100%",
            boxSizing: "border-box",
          },
          "&:has(> [data-mobile-primary-composer='true'][data-mobile-keyboard-open='true'] [data-mobile-editor-area]:focus-within)":
            {
              flex: "0 1 auto",
              overflow: "hidden",
            },
          // Keyboard Focus Mode is a single floating writing surface. Keep the
          // auxiliary state mounted so Plan/Queue/Draft disclosure and edit
          // ownership survive, but remove it from presentation while the main
          // Composer owns the visible software keyboard.
          "&:has(> [data-mobile-primary-composer='true'][data-mobile-keyboard-open='true'] [data-mobile-editor-area]:focus-within) > [data-composer-stack-slot]:not([data-composer-stack-slot='primary'])":
            {
              display: "none",
            },
          // A Queue/Draft edit follows the same focus model. Its containing
          // scrollport must stay mounted because it owns the transaction, so
          // hide Plan and the inactive sibling panel instead of the scrollport.
          "&:has([data-mobile-pending-editor='true'][data-mobile-keyboard-open='true']:focus-within) > [data-composer-stack-slot]:not([data-composer-stack-slot='pending'])":
            {
              display: "none",
            },
          "&:has([data-mobile-pending-editor='true'][data-mobile-keyboard-open='true']:focus-within) [data-mobile-pending-panel]:not([data-mobile-floating-edit='true'])":
            {
              display: "none",
            },
        }),
        // Column mode: a fill-height flex column (queued/drafts on top, the editor
        // card flex:1 below) instead of a bottom-floating stack. No safe-area
        // bottom inset (the AppStatusBar footer owns the column's bottom edge) and
        // overflow hidden so each region scrolls internally, never the column.
        ...(desktop && column && {
          height: "100%",
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          p: 0,
        }),
        ...(desktop && {
          // Desktop is a focus-driven workspace, not the Mobile stacked touch
          // scroller. Keep auxiliary headers visible, but release inactive list
          // height while any Prompt workspace owns focus. Composer therefore
          // opens as a real writing canvas; Plan/Queue/Drafts expand on demand.
          "&:has([data-desktop-region='prompt.plan'][data-desktop-focused='true'], [data-desktop-region='prompt.queued'][data-desktop-focused='true'], [data-desktop-region='prompt.draft'][data-desktop-focused='true'], [data-desktop-region='prompt.composer'][data-desktop-focused='true']) [data-desktop-aux-list]":
            {
              maxHeight: 0,
              paddingTop: 0,
              paddingBottom: 0,
            },
          "& [data-desktop-region][data-desktop-focused='true'] [data-desktop-aux-list]":
            {
              maxHeight: "min(52vh, 640px)",
            },
          "& [data-desktop-region='prompt.plan'][data-desktop-focused='true'] [data-desktop-aux-list]":
            {
              maxHeight: "min(46vh, 560px)",
              paddingBottom: "4px",
            },
          "& [data-desktop-region='prompt.queued'][data-desktop-focused='true'] [data-desktop-aux-list], & [data-desktop-region='prompt.draft'][data-desktop-focused='true'] [data-desktop-aux-list]":
            {
              paddingBottom: "4px",
            },
        }),
      }}
    >
      {
        /* Account quota is independent of ACP turn completion. Claude can keep
          a prompt open while its native retry loop waits for a reset, so surface
          the known exhausted window even though no transcript error exists. */
      }
      <ProviderQuotaStatus
        sessionId={sessionId}
        provider={provider}
        {...(providerVersion === undefined ? {} : { providerVersion })}
        {...(providerDigest === undefined ? {} : { providerDigest })}
        status={status}
        desktop={desktop}
      />
      {
        /* Turn status remains an ordinary stack slot, not a second absolute
          layer. A pending tool-permission outranks the turn-status pill (a
          blocking decision beats a status), and the two remain mutually
          exclusive. */
      }
      {pendingPermission && !approvalDocked
        ? (
          <PermissionOverlay
            item={pendingPermission}
            waiting={timelineState.pendingPermissionCount}
            sessionId={sessionId}
            {...(desktop ? { desktop: DESKTOP_APPROVAL_SLOTS } : {})}
          />
        )
        : callApproval && !approvalDocked
        ? (
          <CallApprovalOverlay
            approval={callApproval}
            sessionId={sessionId}
            {...(desktop ? { desktop: DESKTOP_APPROVAL_SLOTS } : {})}
          />
        )
        : !column
        ? (
          <TurnStatusOverlay
            sessionId={sessionId}
            status={status}
            working={turnWorking}
            paused={sessionState.paused}
            onFocusComposer={(): void => editorRef.current?.focus()}
          />
        )
        : (
          null
        )}
      {
        /* Agent plan: a pinned, collapsible progress summary above Pending and
          Composer. Hidden when there is no plan, when dismissed, or when a
          finished plan has been superseded by a new turn (see showPlan). */
      }
      {
        /* Managed calls this session delegated to other Providers. Hidden
          until the parent has at least one call. */
      }
      <Box
        data-composer-stack-slot="calls"
        data-mobile-input-context={!desktop ? "calls" : undefined}
        {...(desktop
          ? {
            "data-desktop-region": "prompt.calls",
            tabIndex: -1,
          }
          : {})}
        sx={{ "&:empty": { display: "none" } }}
      >
        {callsDockLegacy
          ? (
            <ManagedCallsDock
              sessionId={sessionId}
              desktop={desktop}
              shortcut={desktop
                ? (
                  <Suspense fallback={null}>
                    <DesktopRegionShortcut
                      shortcut={DESKTOP_FOCUS_CALLS_SHORTCUT}
                      title="Focus Calls"
                      singleKeycap={DESKTOP_FOCUS_CALLS_SHORTCUT}
                    />
                  </Suspense>
                )
                : undefined}
            />
          )
          : (
            <BackgroundTasksBar
              sessionId={sessionId}
              desktop={desktop}
              shortcut={desktop
                ? (
                  <Suspense fallback={null}>
                    <DesktopRegionShortcut
                      shortcut={DESKTOP_FOCUS_CALLS_SHORTCUT}
                      title="Background tasks"
                      singleKeycap={DESKTOP_FOCUS_CALLS_SHORTCUT}
                    />
                  </Suspense>
                )
                : undefined}
            />
          )}
      </Box>
      {showPlan && plan && (
        <Box
          data-composer-stack-slot="plan"
          data-mobile-input-context={!desktop ? "plan" : undefined}
          {...(desktop
            ? {
              "data-desktop-region": "prompt.plan",
              tabIndex: -1,
            }
            : {})}
        >
          <PlanDock
            entries={plan.entries}
            onDismiss={dismissPlan}
            desktop={desktop}
            shortcut={desktop
              ? (
                <Suspense fallback={null}>
                  <DesktopRegionShortcut
                    shortcut={DESKTOP_FOCUS_PLAN_SHORTCUT}
                    title="Focus Plan"
                    singleKeycap={DESKTOP_FOCUS_PLAN_SHORTCUT}
                  />
                </Suspense>
              )
              : undefined}
          />
        </Box>
      )}
      <ScheduleSheet
        open={scheduleTarget !== null}
        onClose={(): void => setScheduleTarget(null)}
        initial={scheduleTarget?.initial ?? null}
        editing={scheduleTarget?.id !== undefined}
        onCommit={commitSchedule}
        onUnschedule={(): void => {
          if (scheduleTarget?.id !== undefined) {
            void unscheduleDraft(
              sessionId,
              scheduleTarget.id,
            ).catch(() => undefined);
          }
        }}
      />
      {
        /* Queue and drafts stay a stacked pair of headers. Exclusive
          disclosure means only one body is open, so each panel can own its
          own inner scroller instead of a shared outer scrollbar. */
      }
      {(queue.length > 0 || draftList.length > 0) && !desktop && (
        <Box
          data-composer-stack-slot="pending"
          data-mobile-input-context="pending"
          sx={{
            minHeight: 0,
            flexShrink: 0,
            display: "block",
            "& > * + *": {
              mt: "var(--mobile-composer-stack-gap)",
            },
            m: 0,
            p: 0,
          }}
        >
          {
            /* Queued prompts: while the agent is busy, messages stack here and
              drain one per turn-end. */
          }
          {queue.length > 0 && (
            <PendingPanel
              desktop={false}
              keyboardOpen={mobileKeyboardPresentationOpen}
              kind="queued"
              sessionId={sessionId}
              items={queue}
              status={status}
              commands={(): AvailableCommand[] => availableCommands}
              onEditingChange={onMobileQueuedEditingChange}
              onResumeKeyboardSurface={resumeMobileKeyboardSurface}
            />
          )}
          {/* Drafts: parked messages the user holds + activates on demand. */}
          {draftList.length > 0 && (
            <PendingPanel
              desktop={false}
              keyboardOpen={mobileKeyboardPresentationOpen}
              kind="draft"
              sessionId={sessionId}
              items={draftList}
              status={status}
              commands={(): AvailableCommand[] => availableCommands}
              onEditingChange={onMobileDraftEditingChange}
              onResumeKeyboardSurface={resumeMobileKeyboardSurface}
              // Only offer "move" when there's somewhere to move to.
              onMoveDraft={otherSessions.length > 0
                ? (id: string): void => setMoveSrcId(id)
                : undefined}
              onScheduleDraft={(id: string): void =>
                setScheduleTarget({
                  id,
                  initial: draftList.find((d) => d.id === id)?.schedule ?? null,
                })}
            />
          )}
        </Box>
      )}
      {(queue.length > 0 || draftList.length > 0) && desktop && (
        <Box
          data-composer-stack-slot="pending"
          sx={{
            flexShrink: 0,
            minHeight: 0,
          }}
        >
          {queue.length > 0 && (
            <PendingPanel
              desktop
              keyboardOpen={keyboardOpen}
              kind="queued"
              sessionId={sessionId}
              items={queue}
              status={status}
              commands={(): AvailableCommand[] => availableCommands}
            />
          )}
          {draftList.length > 0 && (
            <PendingPanel
              desktop
              keyboardOpen={keyboardOpen}
              kind="draft"
              sessionId={sessionId}
              items={draftList}
              status={status}
              commands={(): AvailableCommand[] => availableCommands}
              onMoveDraft={otherSessions.length > 0
                ? (id: string): void => setMoveSrcId(id)
                : undefined}
              onScheduleDraft={(id: string): void =>
                setScheduleTarget({
                  id,
                  initial: draftList.find((d) => d.id === id)?.schedule ?? null,
                })}
            />
          )}
        </Box>
      )}
      {
        /* CM6 renders token-backed images at their document position. The tray is
          reserved for files and legacy images that have no placement token. */
      }
      {!mobilePendingEditing && compactTrayAttachments.length > 0 && (
        <AttachmentPreviews
          attachments={compactTrayAttachments}
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
      {
        /* Desktop is a flat writing surface inside the Prompt pane. Mobile
          retains its own input card and keyboard material. Both keep this same
          editor owner mounted through width, state and focus changes. */
      }
      <SessionContextCue sessionId={sessionId} desktop={desktop} />
      {handedOff && <SessionHandedOffDock sessionId={sessionId} desktop={desktop} />}
      <Paper
        data-composer-stack-slot="primary"
        {...(surface === "desktop"
          ? {
            "data-desktop-region": "prompt.composer",
            "data-desktop-focus-default": true,
            tabIndex: -1,
          }
          : {})}
        data-mobile-focus-composer={touchInput ? "true" : undefined}
        data-mobile-primary-composer={touchInput ? "true" : undefined}
        data-mobile-keyboard-open={touchInput && mobileKeyboardPresentationOpen
          ? "true"
          : undefined}
        variant={desktop ? "elevation" : "outlined"}
        elevation={0}
        {...fileDrop.handlers}
        sx={{
          position: "relative",
          display: !desktop && mobilePendingEditing ? "none" : "flex",
          flexDirection: "column",
          ...(handedOff && { display: "none" }),
          ...(desktop && {
            border: 0,
            borderRadius: 0,
            boxShadow: "none",
            // Pane header and status line own focus; do not outline the canvas.
            "&:focus-visible, &:focus-within": { outline: "none", boxShadow: "none" },
          }),
          bgcolor: desktop ? "transparent" : column
            ? (t) =>
              alpha(
                t.palette.background.paper,
                t.palette.mode === "dark" ? 0.18 : 0.34,
              )
            : "transparent",
          // Column mode: the card fills the column's remaining height (below the
          // queued/drafts panels) so the editor is always in its tall form.
          ...(desktop && column && {
            flex: 1,
            minHeight: 0,
            overflow: "hidden",
          }),
          ...(touchInput && {
            borderRadius: mobileComposerPanelFrameSx.borderRadius,
            transition:
              `border-color ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, background-color ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, box-shadow ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}`,
            // visualViewport is the keyboard authority on iOS. WebKit can move
            // DOM focus to <body> while its keyboard and caret remain visible
            // (notably after an attachment/IME transition), so :focus-within
            // alone made this card transparent and let the transcript paint
            // legibly through the writing canvas. Keep the material whenever
            // the measured keyboard presentation is active; the more specific
            // editor-focus rule below still owns expansion geometry.
            "&[data-mobile-keyboard-open='true']": {
              ...mobileFocusedComposerSurfaceSx,
              backgroundImage: "none",
            },
            // Do not gate this fill on :focus-within. iOS can move DOM focus to
            // body while the keyboard stays up; the CM6 scroller would then
            // become a transparent hole over the transcript.
            "&[data-mobile-keyboard-open='true'] [data-mobile-editor-area], &[data-mobile-keyboard-open='true'] [data-mobile-action-row], &[data-mobile-keyboard-open='true'] [data-mobile-focus-format-row]":
              {
                bgcolor: mobileFocusedComposerFill,
                backgroundImage: "none",
              },
            "&[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-theme-none, &[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-editor, &[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-scroller, &[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-content":
              {
                bgcolor: mobileFocusedComposerFill,
                backgroundImage: "none",
                WebkitOverflowScrolling: "auto",
              },
            // Only the editor is allowed to promote the compact card. Utility
            // buttons also live inside this Paper, so plain :focus-within can
            // leave a tall, inert canvas after Settings or another action takes
            // focus while the native textarea has already blurred.
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within)":
              {
                flex: "0 1 auto",
                minHeight: 0,
                maxHeight: "100%",
                // Focus changes hierarchy inside the same card. Keep the outer
                // edge fixed so opening the keyboard does not look like a second
                // component replacing the compact composer.
                // Clip the filtered sample to the same rounded card. Without this,
                // WebKit leaves a sharp readable fringe around the glass edge.
                ...mobileFocusedComposerSurfaceSx,
              },
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-editor-area]":
              {
                flex: "0 1 auto",
                maxHeight: "min(42dvh, 22rem)",
                overflow: "hidden",
              },
            // Keep the compact native field content-tight. Filling leftover
            // viewport with height 100% left a blank band under short text
            // (and the Force-push row) whenever the transcript did not occupy
            // the rest of the screen.
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-native-editor]":
              {
                flex: "0 1 auto",
                minHeight: 0,
                height: "auto",
                maxHeight: "100%",
                overflow: "hidden",
                display: "flex",
                flexDirection: "column",
              },
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-native-editor] [data-mobile-native-textarea='true']":
              {
                height: "auto",
                minHeight: 48,
                maxHeight: "100%",
              },
            // An inline image promotes the compact native textarea to CM6.
            // Size that editor to its thumbnail + text, the same way the
            // native field hugs its lines. Stretching CM6 to the leftover
            // viewport left a tall empty canvas around an 80px image.
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-editor-area] .cm-theme-none, &[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-editor-area] .cm-editor":
              {
                flex: "0 1 auto",
                minHeight: 0,
                height: "auto",
                overflow: "hidden",
              },
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-editor-area] .cm-scroller":
              {
                flex: "0 1 auto",
                minHeight: 0,
                height: "auto",
                maxHeight: "min(42dvh, 22rem)",
                overflowX: "hidden",
                overflowY: "auto",
                scrollPaddingBlock: "12px",
                overscrollBehavior: "contain",
                // Ancestor drawer/pager listeners are non-passive. Without
                // pan-y, iOS waits on them and the overflow-y:auto scroller
                // never starts. Do not use -webkit-overflow-scrolling:touch
                // here: that compositor tile hides the native caret (#2).
                touchAction: "pan-y pinch-zoom",
              },
            "&[data-mobile-keyboard-open='true']:has([data-mobile-editor-area]:focus-within) [data-mobile-editor-area] .cm-content":
              {
                minHeight: 0,
              },
            // Two-track chrome is keyboard presentation, not DOM focus.
            // WKWebView can move activeElement to body during IME / 123-pad
            // while the caret stays; requiring :focus-within collapses this
            // row and parks the action rail under the iOS ∧∨✓ accessory.
            "&[data-mobile-keyboard-open='true'] [data-mobile-focus-format-row]":
              {
                maxHeight: 48,
                opacity: 1,
                transform: "translateY(0)",
                pointerEvents: "auto",
                borderTopColor: mobileComposerHairlineColor,
                transition:
                  `max-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, opacity 110ms ease 55ms, transform ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, border-color 120ms ease`,
              },
            "@media (prefers-reduced-motion: reduce)": {
              transition: "none",
            },
          }),
        }}
      >
        {desktop && (
          <Suspense fallback={null}>
            <DesktopRegionShortcut
              shortcut={DESKTOP_FOCUS_PROMPT_SHORTCUT}
              title="Focus prompt editor"
              showWhenPane="prompt"
              hideWhenRegion="prompt.composer"
              sx={{ position: "absolute", top: 10, right: 10, zIndex: 4 }}
            />
          </Suspense>
        )}
        {fileDrop.active && (
          <Box
            data-composer-file-drop="true"
            aria-hidden
            sx={{
              position: "absolute",
              inset: 0,
              zIndex: 5,
              display: "grid",
              placeItems: "center",
              borderRadius: "inherit",
              border: 2,
              borderStyle: "dashed",
              borderColor: "primary.main",
              bgcolor: (t) => alpha(t.palette.primary.main, 0.08),
              color: "primary.main",
              typography: "body2",
              fontWeight: 600,
              // Hit-testing must keep reaching the editor so a drop over text
              // still lands at its exact position.
              pointerEvents: "none",
            }}
          >
            Drop files to attach
          </Box>
        )}
        {
          /* Top-edge resize handle: drag to grow/shrink the editor; dragging past the
          bottom threshold auto-collapses to the compact auto-grow editor, dragging
          up auto-expands (VSCode-terminal feel). A bigger hit area + an always-
          visible grab-pill so it's findable. Shown unless this is the fullscreen
          sheet (composeFs). Desktop only — the resizable ComposerEditor is the
          !touchInput branch; touch uses the compact editor + the fullscreen ↗.
          Dropped in column mode — the column height IS the editor size. */
        }
        {!touchInput && !composeFs && !column && (
          <Box
            onPointerDown={onResizeStart}
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize editor"
            sx={{
              position: "absolute",
              top: -9,
              left: 0,
              right: 0,
              height: 18,
              cursor: "ns-resize",
              touchAction: "none",
              zIndex: 6,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              // An always-faintly-visible grab-pill (brightens on hover) so it reads
              // as draggable instead of an invisible hot-zone.
              "&::after": {
                content: '""',
                width: 44,
                height: 4,
                borderRadius: 2,
                bgcolor: "text.disabled",
                opacity: 0.45,
                transition: "opacity .15s, background-color .15s, width .15s",
              },
              "&:hover::after": {
                opacity: 0.95,
                bgcolor: "text.secondary",
                width: 56,
              },
            }}
          />
        )}
        {mobilePromptSuggestion !== null && (
          <Box sx={{ px: 1, pt: 0.75, minWidth: 0 }}>
            <Chip
              size="small"
              variant="outlined"
              data-prompt-suggestion="chip"
              aria-label={`Use suggested prompt: ${mobilePromptSuggestion}`}
              label={mobilePromptSuggestion}
              // Keep a focused textarea the first responder (no keyboard
              // bounce); the tap itself focuses an unfocused one.
              onMouseDown={(event): void => event.preventDefault()}
              onClick={(): void => {
                editorRef.current?.insertText(mobilePromptSuggestion);
                dismissPromptSuggestion(sessionId);
              }}
              sx={{ maxWidth: "100%", color: "text.secondary" }}
            />
          </Box>
        )}
        {!composeFs && (
          // PlatformComposerEditor owns the compact-touch hybrid: native textarea
          // for UIKit long-press editing while there is no inline image token,
          // CM6 as soon as a token needs its widget. The fullscreen touch surface
          // follows the same rule so its distant blank canvas remains a reliable
          // native Paste-menu target.
          <Box
            ref={editorAreaRef}
            data-mobile-editor-area={touchInput ? "true" : undefined}
            onPointerDown={touchInput
              ? (): void => {
                if (!mobileKeyboardDismissed) return;
                setMobileKeyboardDismissed(false);
                clearMobileKeyboardDismissed();
              }
              : undefined}
            sx={{
              display: "flex",
              flexDirection: "column",
              minHeight: 0,
              ...(touchInput && {
                // Resting compose only has the fullscreen control in its right
                // rail, so one 44px touch target plus the card edge is enough.
                // The keyboard-only hide control joins that rail in the focused
                // state above, which promotes this to the two-control height.
                // Keeping the resting height content-tight prevents a permanent
                // blank "padding" band regardless of Plan/Queue/Draft presence.
                minHeight: mobileComposerIdleEditorMinHeight,
                transition:
                  `min-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}`,
                "@media (prefers-reduced-motion: reduce)": {
                  transition: "none",
                },
              }),
              ...(desktop && column && { flex: 1 }),
            }}
          >
            <PlatformComposerEditor
              ref={editorRef}
              autoFocus={autoFocus}
              // CM6 is seeded once and owns its document on every surface. Feeding
              // live React state back here would bounce the caret and corrupt IME.
              value={initialDraftText.current}
              {
                // The native compact touch branch is a controlled textarea and needs
                // the live value. PlatformComposerEditor never feeds this back to CM6.
                ...(touchInput ? { nativeValue: text } : {})
              }
              onChange={setText}
              onSubmit={submitAndNotify}
              onSaveDraft={saveDraft}
              borderless
              expanded={inlineExpanded}
              heightPx={inlineExpanded ? composerHeight : 0}
              // Column layout: stretch to fill the column instead of the vh-bounded
              // compact/expanded sizes (overrides expanded/heightPx).
              fill={desktop && column}
              flushRightScrollbar={desktop && column}
              // Reserve a top-right gutter so no line runs under the ↗/↙ expand
              // toggle the card overlays at its top-right corner.
              endInset={desktop && column ? 0 : 36}
              // Keyboard and pointer activation share the same delivery picker.
              holdToForce={false}
              onForceHold={(): void => {
                if (!sendable || queueBtnRef.current === null) return;
                haptic();
                openDeliveryPicker();
              }}
              sessionId={sessionId}
              commands={(): AvailableCommand[] => availableCommands}
              placeholder={preparing
                ? "You can start typing while this session prepares…"
                : messagePlaceholder}
              // Vim is desktop-only — never load it on touch (no physical keyboard /
              // modal editing). ComposerEditor also gates the actual vim import on a
              // fine pointer, so this is belt-and-suspenders.
              vim={touchInput ? false : vim}
              onVimMode={setVimMode}
              onPasteFiles={(files): void =>
                addFiles(files, { preserveFocus: true })}
              promptSuggestion={desktop ? offeredPrompt : null}
              onPromptSuggestionDismiss={(): void =>
                dismissPromptSuggestion(sessionId)}
            />
          </Box>
        )}
        {
          /* Expand toggle, top-right INSIDE the card. DESKTOP: Zed-style inline
            expand — toggles a taller editor in place (flows through the
            floating-stack ResizeObserver). MOBILE: space is tight inline, so ↗ goes
            straight to the FULLSCREEN compose sheet (the first-class long-form /
            future-markdown editor). The editor reserves a right gutter (endInset)
            so text never runs under it. Glyph sized at the BUTTON level so it
            beats the global MuiIconButton `& .MuiSvgIcon-root: 1.5rem` override
            (a per-icon sx loses that specificity); rem so it tracks the font scale.
            Dropped in column mode — the editor already fills the column. */
        }
        {!column && (touchInput
          ? (
            <MobileComposerFixedActionSlot region="primary" overlay>
              <Tooltip title="Fullscreen editor">
                <IconButton
                  size="small"
                  aria-label="fullscreen editor"
                  sx={{
                    ...TOOLBAR_ICON_BTN,
                    color: "text.secondary",
                    "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
                  }}
                  onPointerDown={(event): void => event.preventDefault()}
                  onMouseDown={(event): void => event.preventDefault()}
                  onClick={(): void => {
                    // Snapshot the logical selection before replacing the compact
                    // editor. Pointer-down keeps the old editor focused, then the
                    // same tap mounts and focuses fullscreen exactly once so UIKit
                    // retains both its keyboard transaction and caret/range.
                    const selection = editorRef.current?.getSelection();
                    beginMobileEditorFocusTransfer();
                    flushSync(() => setComposeFs(true));
                    if (selection) {
                      editorRef.current?.focusSelection(selection);
                    } else editorRef.current?.focusEnd();
                  }}
                >
                  <OpenInFull />
                </IconButton>
              </Tooltip>
            </MobileComposerFixedActionSlot>
          )
          : (
            <Tooltip title={expanded ? "Collapse editor" : "Expand editor"}>
              <IconButton
                data-desktop-item-action="default"
                size="small"
                aria-label={expanded ? "collapse editor" : "expand editor"}
                onClick={toggleComposerExpanded}
                sx={{
                  position: "absolute",
                  top: 2,
                  right: 2,
                  zIndex: 1,
                  color: "text.secondary",
                  "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
                }}
              >
                {expanded ? <CloseFullscreen /> : <OpenInFull />}
              </IconButton>
            </Tooltip>
          ))}
        {
          /* Desktop owns persistent editing and delivery rows below the
            scrollport. Mobile retains its keyboard-aware input-card controls. */
        }
        {desktop
          ? (
            <Suspense fallback={<Box sx={{ minHeight: 80 }} />}>
              <DesktopComposerToolbar
                sessionId={sessionId}
                status={status}
                editorRef={editorRef}
                sendButtonRef={queueBtnRef}
                canInsert={!dead}
                sendable={sendable}
                canJumpFront={queue.length > 0}
                canForce={forceable}
                canCurrentTurn={currentTurnAvailable}
                onCurrentTurn={openDeliveryPicker}
                pending={submitFeedback.pending}
                progress={submitFeedback.progress}
                sendLabel={busy || queue.length > 0
                  ? "Send…"
                  : starting || paused || !serverConnected
                  ? "Enqueue"
                  : "Send"}
                sendDescription={!serverConnected
                  ? "Queue until Cowboy is reachable"
                  : busy || queue.length > 0
                  ? "Choose delivery timing and queue position"
                  : paused
                  ? "Queue until the paused queue is resumed"
                  : starting
                  ? "Run after the current work finishes"
                  : dead ? "Send to resume this session" : "Send message"}
                unavailableReason={attachments.some((attachment) =>
                    attachment.pending
                  )
                  ? "Wait for attachments to finish preparing"
                  : "Write a message or attach a file first"}
                overlayOpen={deliveryAnchor !== null ||
                  scheduleTarget !== null}
                onAttach={(): void => fileInputRef.current?.click()}
                onSaveDraft={saveDraft}
                onSchedule={(): void =>
                  setScheduleTarget({ id: undefined, initial: null })}
                onJumpFront={jumpToFront}
                onForce={(): void => {
                  openDeliveryPicker();
                }}
                onSubmit={(): void => {
                  requestDelivery();
                }}
              />
            </Suspense>
          )
          : (
            <>
              {touchInput && (
                <Box
                  data-mobile-focus-format-row
                  sx={{
                    order: 2,
                    flexShrink: 0,
                    maxHeight: 0,
                    minHeight: 0,
                    opacity: 0,
                    overflow: "hidden",
                    pointerEvents: "none",
                    transform: "translateY(8px)",
                    borderTop: 1,
                    borderTopColor: "transparent",
                    transition:
                      `max-height ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, opacity 90ms ease, transform ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, border-color 120ms ease`,
                    "@media (prefers-reduced-motion: reduce)": {
                      transition: "none",
                      transform: "none",
                    },
                  }}
                >
                  <MobileComposerEditingBar
                    actions={
                      <MobileComposerFormatActions
                        commandIds={mobileToolbarIds}
                        editorRef={editorRef}
                        onAttach={(): void => fileInputRef.current?.click()}
                        onPasteImages={pasteClipboardImages}
                        onCustomize={(): void => {
                          releaseMobileComposerFocus();
                          setMobileToolbarSettingsOpen(true);
                        }}
                      />
                    }
                    fixedAction={
                      <MobileComposerAccessoryButton
                        title="Hide keyboard"
                        preserveEditorFocus={false}
                        onClick={(): void => {
                          noteMobileKeyboardDismissed();
                          setMobileKeyboardDismissed(true);
                          dismissMobileSoftwareKeyboard();
                          releaseMobileComposerFocus();
                        }}
                      >
                        <KeyboardHide />
                      </MobileComposerAccessoryButton>
                    }
                  />
                </Box>
              )}
              <Box
                data-mobile-action-row={touchInput ? "true" : undefined}
                onPointerDownCapture={mobileActionClickGuard.onPointerDownCapture}
                onClickCapture={mobileActionClickGuard.onClickCapture}
                sx={{
                  order: touchInput ? 1 : undefined,
                  display: "flex",
                  alignItems: "center",
                  flexShrink: 0,
                  minWidth: 0,
                  pb: 0.5,
                  ...TOOLBAR_MIN_H,
                }}
              >
                <Stack
                  ref={mobileActionsRef}
                  data-mobile-scrollable-actions={touchInput
                    ? "true"
                    : undefined}
                  direction="row"
                  alignItems="center"
                  spacing={compact ? 0 : 0.5}
                  sx={{
                    flex: 1,
                    minWidth: 0,
                    px: 0.5,
                    ...(compact && {
                      justifyContent: "flex-start",
                      columnGap: "clamp(2px, 1vw, 5px)",
                      flexWrap: "nowrap",
                      overflowX: "auto",
                      overflowY: "hidden",
                      overscrollBehaviorX: "contain",
                      scrollbarWidth: "none",
                      WebkitMaskImage:
                        mobileActionEdges.left && mobileActionEdges.right
                          ? "linear-gradient(to right, transparent 0, black 16px, black calc(100% - 16px), transparent 100%)"
                          : mobileActionEdges.left
                          ? "linear-gradient(to right, transparent 0, black 16px)"
                          : mobileActionEdges.right
                          ? "linear-gradient(to right, black calc(100% - 16px), transparent 100%)"
                          : "none",
                      maskImage:
                        mobileActionEdges.left && mobileActionEdges.right
                          ? "linear-gradient(to right, transparent 0, black 16px, black calc(100% - 16px), transparent 100%)"
                          : mobileActionEdges.left
                          ? "linear-gradient(to right, transparent 0, black 16px)"
                          : mobileActionEdges.right
                          ? "linear-gradient(to right, black calc(100% - 16px), transparent 100%)"
                          : "none",
                      "&::-webkit-scrollbar": { display: "none" },
                      "@media (min-width: 700px)": {
                        justifyContent: "flex-start",
                        columnGap: 0.5,
                      },
                    }),
                  }}
                >
                  {
                    /* (Vim mode moved OUT of the toolbar into a Zed-style bottom status bar
            below — see the StatusBar at the card's bottom edge.) */
                  }
                  {
                    /* Mobile starts with the useful, state-bearing Compact action instead of
            a one-tap `/` insertion that is easy to hit accidentally. Slash
            completion remains available by typing `/` in the editor. Desktop
            keeps its dedicated slash affordance in the separate toolbar above. */
                  }
                  {!desktop && compactAction && (
                    <Tooltip
                      title={preparing
                        ? AGENT_BOOTING_HINT
                        : compacting
                        ? "Compacting…"
                        : compactContext.refreshing
                        ? "Refreshing context usage…"
                        : compactTooltip(
                          compactContext.used,
                          compactContext.size,
                        )}
                    >
                      <span>
                        <IconButton
                          aria-label="compact conversation"
                          disabled={agentActionsDisabled || compacting}
                          sx={{ ...TOOLBAR_ICON_BTN, p: 0.75 }}
                          onClick={(): void => setCmdConfirm(compactAction)}
                        >
                          <CompactIcon
                            used={compactContext.used}
                            size={compactContext.size}
                            active={compacting}
                          />
                        </IconButton>
                      </span>
                    </Tooltip>
                  )}
                  {
                    /* @ folds out on compact (mobile) — the row is too tight, and typing
            "@" raises the same file picker. Desktop keeps the dedicated button. */
                  }
                  {!compact && (
                    <Tooltip
                      title={preparing
                        ? AGENT_BOOTING_HINT
                        : "Reference a file (@)"}
                    >
                      <span>
                        <IconButton
                          aria-label="reference a file"
                          disabled={agentActionsDisabled}
                          sx={TOOLBAR_ICON_BTN}
                          onClick={(): void =>
                            editorRef.current?.insertTrigger("@")}
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
                  {
                    /* Session-lifecycle Clear action. Compact is mobile's first button above;
            both actions still require confirmation before they run. */
                  }
                  {clearAction && (
                    <Tooltip
                      title={preparing
                        ? AGENT_BOOTING_HINT
                        : "Clear conversation"}
                    >
                      <span>
                        <IconButton
                          aria-label="clear conversation"
                          disabled={agentActionsDisabled}
                          sx={TOOLBAR_ICON_BTN}
                          onClick={(): void => setCmdConfirm(clearAction)}
                        >
                          <CleaningServices fontSize="small" />
                        </IconButton>
                      </span>
                    </Tooltip>
                  )}
                  {
                    /* Spacer (desktop only) → pins the send/queue cluster to the right edge
            while the left group (slash / @ / attach) stays left. On compact the row
            is space-evenly instead, so the spacer would defeat the even spread.
            (Stop + config + auto-scroll moved to the navbar — see SessionControls.) */
                  }
                  {!compact && <Box sx={{ flex: 1 }} />}
                  {
                    /* Send opens delivery choices while busy; startup sends enqueue.
            Long-press opens the same picker and never interrupts directly. */
                  }
                  {busy || starting
                    ? (
                      <Box
                        component="span"
                        sx={{
                          position: "relative",
                          display: "inline-flex",
                          flexShrink: 0,
                        }}
                      >
                        <IconButton
                          ref={queueBtnRef}
                          color="primary"
                          aria-label={busy || queue.length > 0
                            ? "Choose delivery"
                            : "Enqueue message"}
                          aria-haspopup={busy || queue.length > 0 ? "menu" : undefined}
                          aria-expanded={busy || queue.length > 0 ? deliveryAnchor !== null : undefined}
                          disabled={!sendable || submitFeedback.pending}
                          aria-busy={submitFeedback.pending || undefined}
                          sx={{
                            ...MOBILE_SEND_BTN,
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
                          {submitFeedback.progress
                            ? <CircularProgress size="1.125rem" color="inherit" />
                            : <SendDeliveryIcon />}
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
                                "@keyframes lpfill": {
                                  to: { strokeDashoffset: 0 },
                                },
                              }}
                            />
                          </Box>
                        )}
                      </Box>
                    )
                    : (
                      <Tooltip title={`${queue.length > 0 ? "Choose delivery" : "Send"} (${MOD_LABEL}${ENTER_LABEL})`}>
                        <span>
                          <IconButton
                            ref={queueBtnRef}
                            color="primary"
                            aria-label="send"
                            aria-haspopup={queue.length > 0 ? "menu" : undefined}
                            aria-expanded={queue.length > 0 ? deliveryAnchor !== null : undefined}
                            disabled={!sendable || submitFeedback.pending}
                            aria-busy={submitFeedback.pending || undefined}
                            sx={MOBILE_SEND_BTN}
                            // Keep the native textarea as UIKit's first responder, and
                            // commit stationary touch on pointerup when iOS omits the
                            // trailing synthetic click after stopping scroll momentum.
                            onPointerDown={(event): void => {
                              event.preventDefault();
                              sendTap.onPointerDown(event);
                            }}
                            onPointerMove={sendTap.onPointerMove}
                            onPointerUp={sendTap.onPointerUp}
                            onPointerCancel={sendTap.onPointerCancel}
                            onClick={sendTap.onClick}
                          >
                            {submitFeedback.progress
                              ? <CircularProgress size="1.125rem" color="inherit" />
                              : <SendDeliveryIcon />}
                          </IconButton>
                        </span>
                      </Tooltip>
                    )}
                  {
                    /* Secondary actions: Draft, Schedule and Clear. Delivery
            timing and queue position share the send picker. The narrow-phone ⋮ fold
            is gone — moving the session-level controls (config / auto-scroll / Stop)
            out to the navbar freed the room that fold used to reclaim.
            These stay MOUNTED while the session starts — a booting agent greys
            out the controls it owns, it never removes the row's slots. */
                  }
                  <Tooltip title="Save as draft">
                    <span>
                      <IconButton
                        aria-label="save as draft"
                        disabled={!sendable}
                        sx={TOOLBAR_ICON_BTN}
                        onClick={saveDraft}
                      >
                        <EditNoteOutlined fontSize="small" />
                      </IconButton>
                    </span>
                  </Tooltip>
                  {
                    /* Schedule the current content: park it as a draft that the SERVER
        auto-sends at a future time (fires even with every client offline). */
                  }
                  <Tooltip title="定时发送">
                    <span>
                      <IconButton
                        aria-label="schedule send"
                        disabled={!sendable}
                        sx={TOOLBAR_ICON_BTN}
                        onClick={(): void =>
                          setScheduleTarget({
                            id: undefined,
                            initial: null,
                          })}
                      >
                        <Schedule fontSize="small" />
                      </IconButton>
                    </span>
                  </Tooltip>
                  <Tooltip title="Clear composer">
                    <span data-mobile-composer-clear>
                      <IconButton
                        aria-label="clear composer"
                        disabled={!clearable}
                        sx={TOOLBAR_ICON_BTN}
                        onPointerDown={(event): void => event.preventDefault()}
                        onMouseDown={(event): void => event.preventDefault()}
                        onClick={(event): void =>
                          setClearComposerAnchor(event.currentTarget)}
                      >
                        <DeleteOutline fontSize="small" />
                      </IconButton>
                    </span>
                  </Tooltip>
                </Stack>
              </Box>
              {touchInput && (
                <ComposerToolbarSettings
                  open={mobileToolbarSettingsOpen}
                  onClose={(): void => setMobileToolbarSettingsOpen(false)}
                />
              )}
            </>
          )}
        {
          /* (Vim status moved to the app-wide bottom status bar — see App's
            StatusBar at the very bottom of the window, Zed/VSCode style.) */
        }
      </Paper>
      {
        /* Keep this confirmation non-modal. Even with every FocusTrap option
          disabled, MUI Popover still mounts a Modal and applies its document
          lifecycle; on iOS that ends the focused editor's software-keyboard
          session. Popper leaves the native first responder untouched. */
      }
      <Popper
        open={clearComposerAnchor !== null}
        anchorEl={clearComposerAnchor}
        placement="bottom-end"
        modifiers={[
          { name: "offset", options: { offset: [0, 8] } },
          {
            name: "flip",
            options: { fallbackPlacements: ["top-end", "left"] },
          },
          { name: "preventOverflow", options: { padding: 8 } },
        ]}
        sx={{ zIndex: (theme) => theme.zIndex.modal }}
      >
        <ClickAwayListener
          onClickAway={(): void => setClearComposerAnchor(null)}
        >
          <Paper
            role="dialog"
            aria-modal="false"
            aria-label="Clear composer confirmation"
            sx={{ maxWidth: 244, borderRadius: 2, boxShadow: 8 }}
          >
            <Box sx={{ p: 1.5 }}>
              <Typography variant="body2" sx={{ fontWeight: 700 }}>
                Clear this composer?
              </Typography>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ display: "block", mt: 0.5 }}
              >
                Text and attachments in this unsent message will be removed.
              </Typography>
              <Stack
                direction="row"
                spacing={1}
                justifyContent="flex-end"
                sx={{ mt: 1.5 }}
              >
                <Button
                  size="small"
                  color="inherit"
                  onPointerDown={(event): void => event.preventDefault()}
                  onMouseDown={(event): void => event.preventDefault()}
                  onClick={(): void => setClearComposerAnchor(null)}
                  sx={{ textTransform: "none" }}
                >
                  Cancel
                </Button>
                <Button
                  size="small"
                  variant="contained"
                  color="error"
                  onPointerDown={(event): void => event.preventDefault()}
                  onMouseDown={(event): void => event.preventDefault()}
                  onClick={(): void => {
                    importantHaptic();
                    clearComposer();
                    setClearComposerAnchor(null);
                  }}
                  sx={{ textTransform: "none" }}
                >
                  Clear all
                </Button>
              </Stack>
            </Box>
          </Paper>
        </ClickAwayListener>
      </Popper>
      {
        /* One-shot delivery actions share the same picker. */
      }
      <DeliveryPicker
        sessionId={sessionId}
        anchor={deliveryAnchor}
        busy={busy}
        steerReady={currentTurnAvailable}
        paused={paused}
        queueCount={queue.length}
        onClose={closeDeliveryPicker}
        onNextTurn={(): void => {
          if (!deliverySourceCurrent()) {
            closeDeliveryPicker();
            return;
          }
          setDeliveryAnchor(null);
          submitWithFeedback(() => setComposeFs(false));
        }}
        onQueueFront={(): void => {
          if (!deliverySourceCurrent()) {
            closeDeliveryPicker();
            return;
          }
          setDeliveryAnchor(null);
          jumpToFront();
        }}
        onSteer={(target, nextCheckpoint): void => {
          if (!deliverySourceCurrent()) {
            closeDeliveryPicker();
            return;
          }
          setDeliveryAnchor(null);
          addToCurrentTurn(target, nextCheckpoint);
        }}
        onSendAndPause={(): void => {
          if (!deliverySourceCurrent()) {
            closeDeliveryPicker();
            return;
          }
          setDeliveryAnchor(null);
          void submitFeedback.run(async () => {
            await pauseQueueConfirmed(sessionId);
            // The editor may change while the pause is acknowledged.
            if (!deliverySourceCurrent()) return;
            const confirmation = submitTracked();
            if (confirmation === null) return;
            await confirmation;
            dismissAfterMobileDelivery();
            setComposeFs(false);
            onSubmitted?.();
          });
        }}
        onStop={(): void => {
          setDeliveryAnchor(null);
          setCancelOpen(true);
        }}
      />
      {
        /* Inline-image actions are deliberately non-modal. MUI Popover is built on
          Modal/FocusTrap, which steals CM6 focus and closes the iOS keyboard as
          soon as the image is tapped. Popper keeps the active editor and native
          selection alive while still positioning this toolbar by the image. */
      }
      <Popper
        open={imgSel !== null}
        anchorEl={imgSel?.el ?? null}
        placement="top"
        modifiers={[
          { name: "offset", options: { offset: [0, 8] } },
          {
            name: "flip",
            options: { fallbackPlacements: ["bottom", "right", "left"] },
          },
          { name: "preventOverflow", options: { padding: 8 } },
        ]}
        sx={{ zIndex: (t) => t.zIndex.tooltip }}
      >
        <ClickAwayListener onClickAway={closeImgSel}>
          <Paper
            role="toolbar"
            aria-label="Image actions"
            sx={{
              borderRadius: 999,
              overflow: "hidden",
              border: 1,
              borderColor: "divider",
              boxShadow: 8,
              backgroundImage: "none",
              bgcolor: (t) => alpha(
                t.palette.background.paper,
                t.palette.mode === "dark" ? 0.74 : 0.82,
              ),
              backdropFilter: "blur(16px) saturate(180%)",
              WebkitBackdropFilter: "blur(16px) saturate(180%)",
            }}
          >
            <Stack
              direction="row"
              alignItems="stretch"
              divider={
                <Divider orientation="vertical" flexItem sx={{ my: 0.875 }} />
              }
            >
              <Button
                color="inherit"
                startIcon={<Visibility />}
                onPointerDown={imagePreviewTap.onPointerDown}
                onPointerMove={imagePreviewTap.onPointerMove}
                onPointerUp={imagePreviewTap.onPointerUp}
                onPointerCancel={imagePreviewTap.onPointerCancel}
                onMouseDown={(event): void => event.preventDefault()}
                onClick={imagePreviewTap.onClick}
                sx={INLINE_IMAGE_ACTION_BUTTON_SX}
              >
                Preview
              </Button>
              <Button
                color="error"
                startIcon={<DeleteOutline />}
                onPointerDown={imageDeleteTap.onPointerDown}
                onPointerMove={imageDeleteTap.onPointerMove}
                onPointerUp={imageDeleteTap.onPointerUp}
                onPointerCancel={imageDeleteTap.onPointerCancel}
                onMouseDown={(event): void => event.preventDefault()}
                onClick={imageDeleteTap.onClick}
                sx={INLINE_IMAGE_ACTION_BUTTON_SX}
              >
                Delete
              </Button>
            </Stack>
          </Paper>
        </ClickAwayListener>
      </Popper>
      {
        /* Confirm for the session-lifecycle actions. Compact sends the agent's
          slash-command; Clear is a cowboy session RESET (fresh agent context).
          Both meaningfully change context, so neither fires on a bare tap; Clear
          is styled destructive (it discards the agent's memory of the chat). */
      }
      <SessionActionConfirmDialog
        action={cmdConfirm}
        provider={providerName(provider, providerVersion, providerDigest)}
        activeTurn={busy || starting}
        disableRestoreFocus={touchInput && cmdConfirm?.kind === "reset"}
        onClose={(): void => setCmdConfirm(null)}
        onConfirm={confirmSessionAction}
      />
      {
        /* Mobile fullscreen compose (the ↗ on touch). A near-full-screen sheet for
          comfortable long-form writing — the first-class editor + future home of a
          markdown / rich-text toolbar + preview. Shares the composer's `text` +
          attachments; the inline editor is hidden while this is open (xor), so the
          shared editorRef points at the one mounted here. */
      }
      {composeFs && (
        <FullscreenComposer
          // SHARE the composer's editor handle so addFiles/submit/clear hit the
          // fullscreen editor while it's mounted (the inline one is unmounted) —
          // otherwise a pasted image attaches but inserts no inline thumbnail.
          editorRef={editorRef}
          // The expand tap already focused this editor in-gesture (flushSync), which
          // is what ARMS the iOS long-press menu — so skip the programmatic timer
          // focus (it would re-focus over it and could disarm the menu).
          autoFocus={false}
          // Mounts fresh on open, seeding from the CURRENT in-progress text (like
          // the edit overlay) so inline text carries in; markdown stays literal.
          value={text}
          onChange={setText}
          onSubmit={(): void => requestDelivery(() => setComposeFs(false))}
          sendButtonRef={queueBtnRef}
          onSaveDraft={(): void => {
            saveDraft();
            setComposeFs(false);
          }}
          onSchedule={(): void =>
            setScheduleTarget({ id: undefined, initial: null })}
          submitLabel={busy || queue.length > 0 ? "Send…" : starting || paused ? "Enqueue" : "Send"}
          onCollapse={(): void => {
            // Carry fullscreen edits back to the inline editor. The inline editor
            // REMOUNTS on close (it only renders while !composeFs) and seeds from
            // initialDraftText (its uncontrolled seed) — so refresh that ref to the
            // current text first, else closing reverts to the pre-expand text
            // ("展开/收缩 state 不同步"). Can't feed `text` as the inline value: it'd
            // re-apply on every keystroke and bounce the iOS caret (see line ~500).
            initialDraftText.current = text;
            // The fullscreen editor is the current focus owner. Replace it with
            // the compact editor synchronously inside this same user gesture,
            // then restore its exact selection before UIKit ends the keyboard
            // transaction. A delayed effect or focusEnd() loses the user's caret.
            const selection = editorRef.current?.getSelection();
            beginMobileEditorFocusTransfer();
            flushSync(() => setComposeFs(false));
            if (selection) editorRef.current?.focusSelection(selection);
            else editorRef.current?.focusEnd();
          }}
          onAttach={(): void => fileInputRef.current?.click()}
          onPasteFiles={(files, selection): void =>
            addFiles(
              files,
              selection === undefined
                ? { preserveFocus: true }
                : { preserveFocus: true, selection },
            )}
          onPasteClipboardImages={pasteClipboardImages}
          sessionId={sessionId}
          commands={(): AvailableCommand[] => availableCommands}
          placeholder={messagePlaceholder}
          sendable={sendable}
          attachmentsSlot={fullscreenTrayAttachments.length > 0
            ? (
              <AttachmentPreviews
                attachments={fullscreenTrayAttachments}
                onRemove={removeAttachment}
              />
            )
            : undefined}
        />
      )}
      {
        /* Stop-confirm for the editor's Esc-to-stop (only fires while busy). The
          navbar's Stop button keeps its own copy in SessionControls; both reuse
          StopConfirmDialog and only one is ever open at a time. */
      }
      <StopConfirmDialog
        open={cancelOpen}
        onClose={(): void => setCancelOpen(false)}
        onConfirm={(): void => {
          importantHaptic();
          cancelTurn(sessionId);
          setCancelOpen(false);
        }}
      />
      {
        /* Move-draft destination picker + undo snackbar. Owned here (not in the
          drafts panel) so the snackbar survives when moving the LAST draft
          unmounts that panel. */
      }
      {moveSrcId !== null && (
        <DraftDestinationPicker
          title="Move draft to…"
          sourceId={sessionId}
          onClose={(): void => setMoveSrcId(null)}
          onPick={(destination): void => {
            moveDraft(sessionId, moveSrcId, destination.id);
            setMoveUndo({ id: moveSrcId, toId: destination.id, toTitle: destination.title });
            setMoveSrcId(null);
          }}
        />
      )}
      <Snackbar
        open={moveUndo !== null}
        autoHideDuration={6000}
        onClose={(_e, reason): void => {
          // Keep it up until autohide / Undo — a stray tap (clickaway) shouldn't
          // snatch the Undo away before the user can reach it.
          if (reason !== "clickaway") setMoveUndo(null);
        }}
        anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
        // Match the app-level mobile snackbar: stay above the bottom navigation
        // and composer actions while respecting the home-indicator safe area.
        sx={{
          bottom: {
            xs: "calc(env(safe-area-inset-bottom, 0px) + 96px)",
            sm: 24,
          },
          width: { xs: "calc(100% - 16px)", sm: "auto" },
          maxWidth: { xs: "calc(100% - 16px)", sm: "100%" },
          "& .MuiSnackbarContent-root": {
            width: { xs: "100%", sm: "auto" },
            minWidth: { xs: 0, sm: 288 },
            color: "text.primary",
            bgcolor: (theme) => alpha(
              theme.palette.background.paper,
              theme.palette.mode === "dark" ? 0.96 : 0.94,
            ),
            backgroundImage: "none",
            border: 1,
            borderColor: "divider",
            boxShadow: (theme) => `0 12px 36px ${
              alpha(
                theme.palette.common.black,
                theme.palette.mode === "dark" ? 0.42 : 0.18,
              )
            }`,
            backdropFilter: "blur(24px) saturate(160%)",
            WebkitBackdropFilter: "blur(24px) saturate(160%)",
          },
        }}
        message={moveUndo ? `Moved to ${moveUndo.toTitle}` : ""}
        action={
          <Stack direction="row" spacing={0.25}>
            <Button
              color="primary"
              size="small"
              onClick={(): void => {
                if (!moveUndo) return;
                navigationHaptic();
                globalThis.dispatchEvent(
                  new CustomEvent("cowboy:transcript-save-viewport"),
                );
                openSession(moveUndo.toId);
                setActiveSessionId(moveUndo.toId);
                setMoveUndo(null);
              }}
              sx={{ textTransform: "none" }}
            >
              Open
            </Button>
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
          </Stack>
        }
      />
    </Box>
  );
}

// Confirm before stopping a running turn — the in-flight turn ends, so a single
// click/Esc must not trigger it. Shared by the navbar's Stop button (SessionControls)
// and the composer editor's Esc-to-stop; each owns its own `open` state, the markup
// is one component. Command/Ctrl+Enter confirms; Esc dismisses (Dialog default).
export function StopConfirmDialog({
  open,
  onClose,
  onConfirm,
}: {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
}): React.JSX.Element {
  useConfirmEnter(open, onConfirm);
  return (
    <ConfirmSheet
      open={open}
      onClose={onClose}
      title="Stop the running turn?"
      actions={
        <>
          <Button
            color="inherit"
            onClick={onClose}
            sx={{ textTransform: "none" }}
          >
            Keep running
            <Kbd keys="Esc" />
          </Button>
          <Button
            color="error"
            variant="contained"
            onClick={onConfirm}
            sx={{ textTransform: "none" }}
          >
            Stop
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </>
      }
    >
      <DialogContentText>
        The agent is still working. Stopping ends the current turn; whatever
        it produced so far stays in the transcript.
      </DialogContentText>
    </ConfirmSheet>
  );
}

// Mobile keeps the combined auto-scroll + Stop controls. Desktop moves Following
// into the Conversation header and Stop into the Prompt's delivery row.
export function AutoScrollAndStop({
  sessionId,
  status,
  projection = "history",
  dense = false,
}: {
  sessionId: string;
  status: Status;
  projection?: TranscriptProjection | undefined;
  dense?: boolean;
}): React.JSX.Element {
  const sticky = useSticky(sessionId);
  const exploreAtTail = useExploreAtTail(sessionId);
  const [cancelOpen, setCancelOpen] = useState(false);
  const busy = status === "busy";
  const following = projection === "history"
    ? sticky
    : sticky && exploreAtTail && busy;
  const size = dense ? "small" : "medium";
  return (
    <>
      {
        /* Auto-scroll / follow toggle. Default ON (primary = following the latest);
          tap while inactive → scroll to bottom + follow again; tap while active →
          stop following. Hover-only tooltip so a tap doesn't pop the bubble. */
      }
      <Tooltip
        title={following
          ? "Auto-scroll: on"
          : projection === "explore"
          ? "Scroll to page bottom"
          : "Auto-scroll: off — tap to follow"}
        disableFocusListener
        disableTouchListener
      >
        <IconButton
          size={size}
          aria-label={following
            ? "auto-scroll on"
            : projection === "explore"
            ? "scroll to page bottom"
            : "auto-scroll off"}
          color={following ? "primary" : "default"}
          onClick={(): void => {
            haptic();
            if (projection === "explore") {
              globalThis.dispatchEvent(
                new CustomEvent("cowboy:explore-current-page-bottom", {
                  detail: { sessionId },
                }),
              );
              return;
            }
            if (sticky) setSticky(sessionId, false);
            else requestStickToBottom(sessionId);
          }}
          sx={dense
            ? {
              width: desktopSize(32),
              height: desktopSize(32),
              "& .MuiSvgIcon-root": { fontSize: glyphSize(18) },
            }
            : undefined}
        >
          <VerticalAlignBottom fontSize={size} />
        </IconButton>
      </Tooltip>
      {
        /* Stop is ALWAYS shown so the row doesn't reflow when a turn starts/ends —
          just disabled (greyed) when there's no running turn. <span> lets the
          Tooltip attach over the disabled button. */
      }
      <Tooltip title="Stop">
        <span>
          <IconButton
            size={size}
            color="error"
            aria-label="cancel"
            disabled={!busy}
            onClick={(): void => setCancelOpen(true)}
            sx={dense
              ? {
                width: desktopSize(32),
                height: desktopSize(32),
                "& .MuiSvgIcon-root": { fontSize: glyphSize(18) },
              }
              : undefined}
          >
            <Stop fontSize={size} />
          </IconButton>
        </span>
      </Tooltip>
      <StopConfirmDialog
        open={cancelOpen}
        onClose={(): void => setCancelOpen(false)}
        onConfirm={(): void => {
          importantHaptic();
          cancelTurn(sessionId);
          setCancelOpen(false);
        }}
      />
    </>
  );
}

// The SESSION-level controls in the navbar: the agent-config ⊟ (options sheet),
// plus — ON MOBILE — the auto-scroll + Stop pair. On desktop those two live in the
// bottom status bar instead (AppStatusBar), so here they're gated to touch.
// Store-driven, so the navbar renders it from just the active session's id + status.
export function SessionControls({
  sessionId,
  status,
  projection,
  onProjectionChange,
}: {
  sessionId: string;
  status: Status;
  projection?: TranscriptProjection | undefined;
  onProjectionChange?: ((projection: TranscriptProjection) => void) | undefined;
}): React.JSX.Element {
  const configOptions = useStoreSelector((snapshot) =>
    snapshot.configOptions.get(sessionId) ?? EMPTY_CONFIG_OPTIONS
  );
  const session = useStoreSelector(
    (snapshot) =>
      snapshot.sessions.find((candidate) => candidate.id === sessionId),
    sameComposerSheetSession,
  );
  const timelineState = useStoreSelector(
    (snapshot) => composerTimelineSlice(snapshot.timelines.get(sessionId)),
    sameComposerTimelineSlice,
  );
  const touchInput = useTouchComposer();
  const [sheetOpen, setSheetOpen] = useState(false);
  const [sheetFocus, setSheetFocus] = useState<SessionSettingsFocus>(
    "session",
  );
  useEffect(() => {
    const onOpen = (event: Event): void => {
      if (!sessionSettingsEventMatches(event, sessionId)) return;
      if (touchInput) releaseMobileComposerFocus();
      setSheetFocus(sessionSettingsFocusFromEvent(event));
      setSheetOpen(true);
    };
    const onClose = (event: Event): void => {
      if (sessionSettingsEventMatches(event, sessionId)) setSheetOpen(false);
    };
    // A request made from another session's menu arrives before this mounts.
    const pending = pendingSessionSettings(sessionId);
    if (pending) {
      setSheetFocus(pending);
      setSheetOpen(true);
    }
    document.addEventListener(OPEN_SESSION_SETTINGS_EVENT, onOpen);
    document.addEventListener(CLOSE_SESSION_SETTINGS_EVENT, onClose);
    return () => {
      document.removeEventListener(OPEN_SESSION_SETTINGS_EVENT, onOpen);
      document.removeEventListener(CLOSE_SESSION_SETTINGS_EVENT, onClose);
    };
  }, [touchInput, sessionId]);
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
  const providerUpdate = sessionProviderUpdate(session);
  const optionPresentations = useMemo(
    () =>
      providerConfigOptionPresentations(
        session?.provider,
        session?.provider_version,
        session?.provider_generation_digest,
      ),
    [
      session?.provider,
      session?.provider_generation_digest,
      session?.provider_version,
    ],
  );
  // Same fixed display order as the old inline chip row, so the sheet's selectors
  // never reshuffle between config_option_update notifications.
  const options = useMemo(() => {
    return providerConfigOptions(session?.provider, configOptions).sort(
      (a, b) => {
        const ai = providerConfigOptionOrder(a, optionPresentations.get(a.id));
        const bi = providerConfigOptionOrder(b, optionPresentations.get(b.id));
        if (ai === bi) return a.id.localeCompare(b.id);
        return ai - bi;
      },
    );
  }, [configOptions, optionPresentations, session?.provider]);
  const showSkeleton = !dead && options.length === 0 &&
    (status === "starting" || status === "running");
  const compactAction = useMemo(
    () =>
      resolveSessionAction(
        "compact",
        session?.provider ?? "",
        timelineState.availableCommands,
        session?.provider_version,
        session?.provider_generation_digest,
      ),
    [
      session?.provider,
      session?.provider_version,
      session?.provider_generation_digest,
      timelineState.availableCommands,
    ],
  );
  const clearAction = useMemo(
    () =>
      resolveSessionAction(
        "clear",
        session?.provider ?? "",
        timelineState.availableCommands,
        session?.provider_version,
        session?.provider_generation_digest,
      ),
    [
      session?.provider,
      session?.provider_version,
      session?.provider_generation_digest,
      timelineState.availableCommands,
    ],
  );
  const runSessionAction = (action: SessionAction): Promise<void> => {
    haptic();
    if (action.kind === "reset") return resetSession(sessionId);
    if (action.command !== undefined) {
      return submitPrompt(sessionId, action.command, []);
    }
    return Promise.resolve();
  };
  return (
    <>
      {session && (
        <Tooltip
          title={providerUpdate
            ? `Options · ${providerUpdate.version} update available`
            : "Options"}
        >
          <IconButton
            aria-label={providerUpdate
              ? "options, Provider update available"
              : "options"}
            onPointerDown={(event): void => {
              // Session settings lives in the navbar, outside the Composer's
              // focus region. Release the active Mobile editor while the
              // pointer gesture still owns it; by click time WebKit may have
              // focused this button and hidden the keyboard without clearing
              // the Composer's :focus-within geometry.
              if (event.pointerType === "touch") {
                event.currentTarget.dataset.touchActivated = "true";
              } else if (event.pointerType === "mouse") {
                delete event.currentTarget.dataset.touchActivated;
              }
              if (touchInput) releaseMobileComposerFocus();
            }}
            onPointerEnter={(event): void => {
              if (event.pointerType === "mouse") {
                delete event.currentTarget.dataset.touchActivated;
              }
            }}
            onKeyDown={(event): void => {
              delete event.currentTarget.dataset.touchActivated;
            }}
            onClick={(event): void => {
              // Keyboard navigation does not emit pointerdown. Keep the same
              // transition boundary for accessibility and hardware keyboards.
              if (touchInput) releaseMobileComposerFocus();
              setSheetFocus("session");
              setSheetOpen(true);
              // The sheet's footer dismiss mounts under this tap. Blur so
              // WebKit cannot leave :hover/:focus paint on the trigger or
              // retarget it onto the new close control at the bottom.
              if (touchInput) event.currentTarget.blur();
            }}
            sx={{
              // Same latch as the hamburger / Code buttons: iOS synthesizes
              // :hover after a finger tap and leaves this navbar control
              // looking selected, which then reads as a highlighted bottom
              // chrome once the session sheet covers it.
              "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible":
                {
                  bgcolor: "transparent",
                },
              "&[data-touch-activated='true']:active": {
                bgcolor: "action.selected",
              },
            }}
          >
            <Badge
              variant="dot"
              color="info"
              overlap="circular"
              invisible={providerUpdate === null}
              data-provider-update-badge={providerUpdate !== null || undefined}
            >
              <Tune />
            </Badge>
          </IconButton>
        </Tooltip>
      )}
      {
        /* Auto-scroll + Stop ride the navbar on MOBILE; desktop puts them in the
          bottom status bar instead (see AppStatusBar), so gate them to touch here. */
      }
      {touchInput && (
        <AutoScrollAndStop
          sessionId={sessionId}
          status={status}
          projection={projection}
        />
      )}
      {
        /* Portal to <body>: SessionControls lives inside the navbar (a low z-index
          stacking context), but the mobile config sheet is a NON-portaled DetentSheet
          (position:fixed + zIndex.modal, rendered inline). Mounted in the navbar it
          would be trapped BELOW the floating composer (its own higher stacking
          context), which then bleeds through the open sheet. Portaling lifts it to the
          top stacking context so it covers everything, like every other app sheet. */
      }
      {createPortal(
        <ComposerSheet
          open={sheetOpen}
          focus={sheetFocus}
          onClose={(): void => {
            setSheetOpen(false);
            setSheetFocus("session");
          }}
          session={session}
          options={options}
          loading={showSkeleton}
          status={status}
          compacting={status === "busy" && timelineState.compactingTail}
          compactAction={compactAction}
          clearAction={clearAction}
          onSessionAction={runSessionAction}
          projection={projection}
          onProjectionChange={onProjectionChange}
          onApplyPreset={(changes): Promise<void> =>
            setSessionConfigOptions(sessionId, changes)}
          onSelectOption={(configId, value): boolean =>
            send({
              type: "set_config_option",
              session_id: sessionId,
              config_id: configId,
              value,
            })}
        />,
        document.body,
      )}
    </>
  );
}

function RecommendedRunConfigPresetButton({
  preset,
  selected,
  disabled,
  pending,
  progress,
  onActivate,
}: {
  preset: RunConfigPreset;
  selected: boolean;
  disabled: boolean;
  pending: boolean;
  progress: boolean;
  onActivate: () => void;
}): React.JSX.Element {
  const activateTap = useReliableTouchTap<HTMLButtonElement>(onActivate);

  return (
    <ButtonBase
      {...activateTap}
      disabled={disabled}
      aria-pressed={selected}
      aria-busy={pending || undefined}
      data-run-config-preset={preset.id}
      sx={{
        minHeight: 58,
        px: 1.5,
        py: 1,
        // Match the theme's 10px control radius used by the other actions in
        // this settings sheet.
        borderRadius: 1,
        border: 1,
        borderColor: selected ? "primary.main" : "divider",
        bgcolor: (theme) =>
          selected
            ? alpha(theme.palette.primary.main, 0.13)
            : alpha(theme.palette.background.default, 0.34),
        textAlign: "left",
        justifyContent: "flex-start",
        touchAction: "manipulation",
        "&:active": { transform: "scale(0.99)" },
        "&.Mui-disabled": { opacity: 0.46 },
      }}
    >
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Stack direction="row" spacing={0.75} alignItems="center">
          <Typography variant="body2" sx={{ fontWeight: 750 }}>
            {preset.name}
          </Typography>
          {preset.isDefault && (
            <Chip
              label="Default preset"
              size="small"
              color="primary"
              variant="outlined"
              sx={{ height: 20, fontSize: "0.625rem" }}
            />
          )}
        </Stack>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: "block", mt: 0.2 }}
        >
          {preset.detail}
        </Typography>
      </Box>
      <Box
        aria-hidden
        sx={{
          width: 18,
          height: 18,
          display: "grid",
          placeItems: "center",
          flexShrink: 0,
        }}
      >
        {progress ? (
          <CircularProgress size={desktopSize(14)} thickness={4.5} color="inherit" />
        ) : selected ? <Check sx={{ fontSize: glyphSize(18) }} /> : null}
      </Box>
    </ButtonBase>
  );
}

// Unified bottom sheet for touch viewports: every action lives here, none
// in a visible inline row. ChatGPT / DeepSeek / Gemini all collapse their
// composer controls behind a single `+` because chip rows wrap awkwardly
// on iPad portrait (820px) and break entirely on a 390px iPhone, while
// the bottom-sheet pattern is iOS-native muscle memory.
function ComposerSheet({
  open,
  focus = "session",
  onClose,
  session,
  options,
  loading,
  status,
  compacting,
  compactAction,
  clearAction,
  onSessionAction,
  projection,
  onProjectionChange,
  onApplyPreset,
  onSelectOption,
}: {
  open: boolean;
  focus?: SessionSettingsFocus;
  onClose: () => void;
  session: SessionMeta | undefined;
  options: ConfigOption[];
  loading: boolean;
  status: Status;
  compacting: boolean;
  compactAction: SessionAction | null;
  clearAction: SessionAction | null;
  onSessionAction: (action: SessionAction) => Promise<void>;
  projection?: TranscriptProjection | undefined;
  onProjectionChange?: ((projection: TranscriptProjection) => void) | undefined;
  onApplyPreset: (changes: readonly ConfigOptionChange[]) => Promise<void>;
  onSelectOption: (configId: string, value: string | boolean) => boolean;
}): React.JSX.Element {
  // Phones and portrait touch tablets keep the bottom sheet. Pointer-driven
  // devices from 768px and every viewport >=1024px get a centered dialog.
  const useSheetSurface = useMediaQuery(
    "(max-width: 767.95px), (min-width: 768px) and (max-width: 1023.95px) and (pointer: coarse)",
  );
  const recommendedPresets = runConfigPresets(
    session?.provider,
    options,
    session?.provider_version,
    session?.provider_generation_digest,
  );
  const optionPresentations = useMemo(
    () =>
      providerConfigOptionPresentations(
        session?.provider,
        session?.provider_version,
        session?.provider_generation_digest,
      ),
    [
      session?.provider,
      session?.provider_generation_digest,
      session?.provider_version,
    ],
  );
  const authoritativeActivePreset = activeRunConfigPreset(
    recommendedPresets,
    options,
  );
  const [pendingPresetId, setPendingPresetId] = useState<string | null>(null);
  const presetAction = useNetworkActionState();
  const pendingPreset = pendingPresetId === null
    ? undefined
    : recommendedPresets.find((preset) => preset.id === pendingPresetId);
  const currentConfigSummary = runConfigSummary(options);
  const currentConfigTitle = runConfigCurrentTitle(
    authoritativeActivePreset?.name,
  );
  const [customizeAgent, setCustomizeAgent] = useState(false);
  const agentDetailsId = useId();
  const [sessionActionsExpanded, setSessionActionsExpanded] = useState(false);
  const [workspaceOptionsExpanded, setWorkspaceOptionsExpanded] = useState(
    false,
  );
  const [cmdConfirm, setCmdConfirm] = useState<SessionAction | null>(null);
  const [reloadConfirm, setReloadConfirm] = useState(false);
  const [updateConfirm, setUpdateConfirm] = useState(false);
  const closeUpdateConfirm = useCallback(() => setUpdateConfirm(false), []);
  useEffect(() => {
    if (open) {
      setUpdateConfirm(false);
      setCustomizeAgent(false);
      setPendingPresetId(null);
      setSessionActionsExpanded(false);
      setWorkspaceOptionsExpanded(false);
      setCmdConfirm(null);
      setReloadConfirm(false);
    }
  }, [open, session?.id]);
  useEffect(() => {
    if (!open || focus !== "agent") return undefined;
    const frame = globalThis.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("[data-session-settings-agent]")
        ?.scrollIntoView({ block: "start", behavior: "smooth" });
    });
    return () => globalThis.cancelAnimationFrame(frame);
  }, [focus, open, session?.id]);
  const showAgentDetails = recommendedPresets.length === 0 || customizeAgent;
  const displayTitle = session?.title.startsWith(`${session.provider} · `)
    ? session.title.slice(session.provider.length + 3)
    : session?.title ?? "";
  const [title, setTitle] = useState(displayTitle);
  const [titleFocused, setTitleFocused] = useState(false);
  const titleInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    setTitle(displayTitle);
    setTitleFocused(false);
  }, [displayTitle, open]);
  const trimmedTitle = title.trim();
  const titleDirty = trimmedTitle !== displayTitle;
  const saveTitle = (): void => {
    if (session && trimmedTitle && titleDirty) {
      renameSession(session.id, trimmedTitle);
    } else if (!trimmedTitle) {
      setTitle(displayTitle);
    }
    titleInputRef.current?.blur();
    setTitleFocused(false);
  };
  const saveTitleAndClose = (): void => {
    if (!trimmedTitle) return;
    saveTitle();
    setSessionActionsExpanded(false);
    setWorkspaceOptionsExpanded(false);
    setCmdConfirm(null);
    setReloadConfirm(false);
    onClose();
  };
  const cancelTitle = (): void => {
    setTitle(displayTitle);
    titleInputRef.current?.blur();
    setTitleFocused(false);
  };
  const close = (): void => {
    setTitle(displayTitle);
    setTitleFocused(false);
    setSessionActionsExpanded(false);
    setWorkspaceOptionsExpanded(false);
    setCmdConfirm(null);
    onClose();
  };
  const confirmSessionAction = async (): Promise<void> => {
    if (cmdConfirm === null) return;
    const action = cmdConfirm;
    setCmdConfirm(null);
    await onSessionAction(action);
  };
  useConfirmEnter(cmdConfirm !== null, () => {
    void confirmSessionAction();
  });
  const editingTitle = titleFocused || titleDirty;
  return (
    <>
      <Sheet
      open={open}
      onClose={close}
      forceSheet={useSheetSurface}
      cover={useSheetSurface}
      wide
      mobileDismiss={useSheetSurface ? "none" : "header"}
      floatingActions={useSheetSurface ? !editingTitle : true}
      actions={useSheetSurface
        ? (
          editingTitle
            ? (
              <MobileDecisionActions
                shelf
                onCancel={cancelTitle}
                confirmLabel="Save"
                onConfirm={saveTitleAndClose}
                confirmDisabled={!trimmedTitle}
                preserveFocus
              />
            )
            : <MobileSheetDismiss onClose={close} />
        )
        : undefined}
      title={useSheetSurface ? undefined : (
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
        >
          <Typography variant="h6" component="span" sx={{ fontWeight: 700 }}>
            Session settings
          </Typography>
          <Box sx={{ position: "relative", display: "inline-flex" }}>
            <IconButton
              aria-label="close session settings"
              onClick={close}
              size="small"
            >
              <Close fontSize="small" />
            </IconButton>
            <Kbd keys="Esc" floating />
          </Box>
        </Stack>
      )}
    >
      {(loading || options.length > 0) && (
        <>
          <Box data-session-settings-agent sx={{ py: 1.5 }}>
            <Typography
              variant="overline"
              color="text.secondary"
              sx={{ letterSpacing: 0.8, lineHeight: 1.6 }}
            >
              Agent
            </Typography>
            {status === "exited" && (
              <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
                Changes are saved for your next message. The session stays asleep.
              </Typography>
            )}
            {session && <ConfigRevertBack sessionId={session.id} options={options} />}
            {loading
              ? (
                <Stack
                  direction="row"
                  spacing={1.5}
                  alignItems="center"
                  sx={{ py: 1, color: "text.secondary" }}
                >
                  <CircularProgress size={desktopSize(16)} />
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
                <Stack spacing={1.25} sx={{ mt: 1.25 }}>
                  {recommendedPresets.length > 0
                    ? (
                      <ButtonBase
                        aria-label={showAgentDetails
                          ? "Collapse current agent configuration"
                          : "Expand current agent configuration"}
                        aria-expanded={showAgentDetails}
                        aria-controls={agentDetailsId}
                        onClick={(): void => {
                          haptic();
                          setCustomizeAgent((value) => !value);
                        }}
                        sx={{
                          width: "100%",
                          minHeight: 44,
                          px: 0.5,
                          borderRadius: 1.5,
                          justifyContent: "space-between",
                          color: "text.secondary",
                          textAlign: "left",
                          "&:active": { bgcolor: "action.hover" },
                        }}
                      >
                        <Box sx={{ minWidth: 0, pr: 1 }}>
                          <Typography
                            variant="body2"
                            sx={{ fontWeight: 650 }}
                          >
                            {currentConfigTitle}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.secondary"
                            sx={{ display: "block", lineHeight: 1.35 }}
                          >
                            {pendingPreset
                              ? `Applying ${pendingPreset.name}…`
                              : currentConfigSummary}
                          </Typography>
                        </Box>
                        <ExpandMore
                          fontSize="small"
                          sx={{
                            flexShrink: 0,
                            transform: showAgentDetails
                              ? "rotate(180deg)"
                              : "none",
                            transition: (theme) =>
                              theme.transitions.create("transform"),
                          }}
                        />
                      </ButtonBase>
                    )
                    : (
                      <Box>
                        <Typography
                          variant="body2"
                          sx={{ fontWeight: 650 }}
                        >
                          {currentConfigTitle}
                        </Typography>
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ display: "block", lineHeight: 1.35 }}
                        >
                          {pendingPreset
                            ? `Applying ${pendingPreset.name}…`
                            : currentConfigSummary}
                        </Typography>
                      </Box>
                    )}
                  <Collapse
                    in={showAgentDetails}
                    id={agentDetailsId}
                    unmountOnExit={recommendedPresets.length > 0}
                  >
                    <Stack
                      spacing={2}
                      sx={{ pt: recommendedPresets.length > 0 ? 0.5 : 0 }}
                    >
                      {options.map((opt) => (
                        <ConfigSheetDropdown
                          key={opt.id}
                          option={opt}
                          disabled={providerConfigOptionDisabled(
                            status,
                            optionPresentations.get(opt.id),
                          )}
                          onSelect={(value): void => {
                            onSelectOption(opt.id, value);
                          }}
                        />
                      ))}
                    </Stack>
                  </Collapse>
                  {recommendedPresets.length > 0 && (
                    <>
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        sx={{ fontWeight: 700 }}
                      >
                        Recommended
                      </Typography>
                      <Box
                        sx={{
                          display: "grid",
                          gridTemplateColumns: {
                            xs: "1fr",
                            sm: "repeat(2, minmax(0, 1fr))",
                          },
                          gap: 1,
                        }}
                      >
                        {recommendedPresets.map((preset) => (
                          <RecommendedRunConfigPresetButton
                            key={preset.id}
                            preset={preset}
                            selected={authoritativeActivePreset?.id ===
                              preset.id}
                            disabled={presetAction.pending}
                            pending={pendingPresetId === preset.id}
                            progress={pendingPresetId === preset.id &&
                              presetAction.progress}
                            onActivate={(): void => {
                              const changes = runConfigPresetChanges(
                                preset,
                                options,
                              );
                              if (changes.length === 0) {
                                setPendingPresetId(null);
                                setCustomizeAgent(false);
                                return;
                              }
                              setPendingPresetId(preset.id);
                              haptic();
                              void presetAction.run(() =>
                                onApplyPreset(changes)
                              ).then((succeeded) => {
                                setPendingPresetId(null);
                                if (succeeded) setCustomizeAgent(false);
                              });
                            }}
                          />
                        ))}
                      </Box>
                    </>
                  )}
                </Stack>
              )}
          </Box>
        </>
      )}
      {session && !isManagedChild(session) && (
        <>
          {/* Right under the Agent block: model and reasoning apply in place;
              another subscription or Provider is a Switch with Back. */}
          <SessionContextSettings session={session} />
          <Divider />
          <SessionToolsSection sessionId={session.id} />
        </>
      )}
      {session && (
        <>
          <Divider />
          <SessionInfoSection
            session={session}
            title={title}
            titleInputRef={titleInputRef}
            onTitleChange={setTitle}
            onTitleFocusChange={setTitleFocused}
            onTitleSave={saveTitle}
            saveOnBlur={!useSheetSurface}
            compactAction={compactAction}
            clearAction={clearAction}
            compacting={compacting}
            actionsExpanded={sessionActionsExpanded}
            onActionsExpandedChange={setSessionActionsExpanded}
            onSessionAction={setCmdConfirm}
            onReload={(): void => setReloadConfirm(true)}
            onProviderUpdate={(): void => setUpdateConfirm(true)}
          />
          <WorkspaceOptionsSection
            session={session}
            expanded={workspaceOptionsExpanded}
            onExpandedChange={setWorkspaceOptionsExpanded}
            projection={projection}
            onProjectionChange={onProjectionChange}
          />
          <SessionProviderSection session={session} />
        </>
      )}
      <SessionActionConfirmDialog
        action={cmdConfirm}
        provider={providerName(
          session?.provider ?? "",
          session?.provider_version,
          session?.provider_generation_digest,
        )}
        activeTurn={session?.status === "busy" ||
          session?.status === "starting"}
        onClose={(): void => setCmdConfirm(null)}
        onConfirm={confirmSessionAction}
      />
      <SessionReloadDialog
        session={open && reloadConfirm ? session : null}
        onClose={(): void => setReloadConfirm(false)}
      />
      <ProviderUpdateDialog
        session={open && updateConfirm ? session : null}
        onClose={closeUpdateConfirm}
      />
      </Sheet>
    </>
  );
}

// Read-only session metadata at the top of the options sheet. This used to
// live behind a long-press on the mobile title bar (a gesture nobody found),
// so it now rides the one popup the user already opens to change mode / model
// / effort. Desktop shows the same facts in the persistent sidebar, so this
// section only renders inside the compact-tier sheet.
interface DeepseekCacheProtectionStatus {
  state: "protected" | "inactive" | "disabled";
  protected_tokens?: number;
  last_verified_at_ms?: number;
  next_attempt_at_ms?: number;
  scheduled_interval_ms?: number;
  base_interval_ms?: number;
  adaptive_interval_ms?: number;
  minimum_interval_ms?: number;
  maximum_interval_ms?: number;
  jitter_ms?: number;
  expires_at_ms?: number;
  attempts?: number;
  minimumHitTokens?: number;
  contextUsed?: number;
}

function compactCacheTokens(tokens: number | undefined): string {
  if (tokens === undefined || !Number.isFinite(tokens) || tokens < 0) return "";
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000).toLocaleString()}K`;
  return tokens.toLocaleString();
}

function compactCacheDuration(milliseconds: number | undefined): string {
  if (
    milliseconds === undefined || !Number.isFinite(milliseconds) ||
    milliseconds <= 0
  ) {
    return "";
  }
  const totalMinutes = Math.max(1, Math.round(milliseconds / 60_000));
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0) return `${minutes}m`;
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
}

function SessionInfoSection({
  session,
  title,
  titleInputRef,
  onTitleChange,
  onTitleFocusChange,
  onTitleSave,
  saveOnBlur,
  compactAction,
  clearAction,
  compacting,
  actionsExpanded,
  onActionsExpandedChange,
  onSessionAction,
  onReload,
  onProviderUpdate,
}: {
  session: SessionMeta;
  title: string;
  titleInputRef: React.RefObject<HTMLInputElement | null>;
  onTitleChange: (title: string) => void;
  onTitleFocusChange: (focused: boolean) => void;
  onTitleSave: () => void;
  saveOnBlur: boolean;
  compactAction: SessionAction | null;
  clearAction: SessionAction | null;
  compacting: boolean;
  actionsExpanded: boolean;
  onActionsExpandedChange: (expanded: boolean) => void;
  onSessionAction: (action: SessionAction) => void;
  onReload: () => void;
  onProviderUpdate: () => void;
}): React.JSX.Element {
  // Title is editable right here — this sheet already shows the session's identity,
  // so the rename (edit-title) belongs with it rather than off in app Settings.
  // Strip the auto "provider · " prefix like the navbar does, so you edit the
  // DISPLAY title, not the machine string. The parent owns the draft because
  // mobile replaces the footer Close action with an explicit Save action while
  // this field is being edited. Plain Enter is intentionally not a commit path.
  const project = sessionProjectLabel(session) ?? "Not recorded";
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  const execution = sessionExecution(session);
  const machineName = (id: string | undefined): string =>
    machines.find((machine) => machine.id === id)?.display_name ?? id ?? "Local";
  const contextUsed = session.context_used ?? 0;
  const contextSize = session.context_size ?? 0;
  const hasContext = contextSize > 0;
  const contextPercent = hasContext
    ? Math.min(100, Math.max(0, contextUsed / contextSize * 100))
    : 0;
  const providerEntry = currentProviderEntry(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const usageAccount = providerEntry?.manifest.host.account_usage?.provider ??
    session.provider;
  const cacheOptionName = usageCacheOptionName(usageAccount);
  const cacheProtectionVisible =
    providerEntry?.manifest.host.features.includes("cache_protection_v1") ===
      true &&
    contextUsed >= usageCacheMinHitTokens(usageAccount);
  const sessionProviderName = providerName(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const [cacheProtection, setCacheProtection] = useState<
    DeepseekCacheProtectionStatus | null
  >(null);
  const [cacheProtectionUnavailable, setCacheProtectionUnavailable] = useState(
    false,
  );
  useEffect(() => {
    if (!cacheProtectionVisible) {
      setCacheProtection(null);
      setCacheProtectionUnavailable(false);
      return;
    }
    const controller = new AbortController();
    const refresh = (): void => {
      void fetch(
        `/api/sessions/${encodeURIComponent(session.id)}/cache-protection`,
        { signal: controller.signal },
      ).then(async (response) => {
        await expectHttpOk(response, "Could not load cache protection status");
        const value = await response.json() as DeepseekCacheProtectionStatus;
        if (!["protected", "inactive", "disabled"].includes(value.state)) {
          throw new Error("invalid cache status");
        }
        setCacheProtection(value);
        setCacheProtectionUnavailable(false);
      }).catch(() => {
        if (!controller.signal.aborted) {
          setCacheProtection(null);
          setCacheProtectionUnavailable(true);
        }
      });
    };
    refresh();
    const interval = globalThis.setInterval(refresh, 60_000);
    return (): void => {
      controller.abort();
      globalThis.clearInterval(interval);
    };
  }, [cacheProtectionVisible, session.id]);
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: "Project", value: project },
    { label: "Working dir", value: execution.cwd, mono: true },
    ...(execution.state === "local" ? [] : [
      { label: "Agent runtime", value: machineName(session.machine_id) },
      { label: "Environment", value: execution.machineId ? machineName(execution.machineId) : "Unavailable" },
    ]),
    { label: "Source", value: originLabel(session.origin) },
    { label: "Status", value: session.status },
    { label: "Session id", value: session.id, mono: true },
  ];
  const cacheBaseInterval =
    compactCacheDuration(cacheProtection?.base_interval_ms) ||
    usageCacheIntervalLabel(usageAccount);
  const cacheAdaptiveInterval = compactCacheDuration(
    cacheProtection?.adaptive_interval_ms,
  );
  const cacheScheduledInterval = compactCacheDuration(
    cacheProtection?.scheduled_interval_ms,
  );
  const actionsPanelId = useId();
  const cacheIntervalDetail = cacheProtection?.state === "protected"
    ? `Base ${cacheBaseInterval} · next ${
      cacheScheduledInterval || cacheAdaptiveInterval || "learning"
    }`
    : `Base ${cacheBaseInterval} · adaptive target ${
      cacheAdaptiveInterval || "learning"
    }`;
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
      <ProviderUpdateCard session={session} onUpdate={onProviderUpdate} />
      <TextField
        inputRef={titleInputRef}
        name="session-title"
        size="small"
        label="Title"
        value={title}
        onChange={(e): void => onTitleChange(e.target.value)}
        onFocus={(e): void => {
          onTitleFocusChange(true);
          // Select the whole title on focus, so tapping it to rename lets you
          // replace it in one go (instead of fiddling a caret into a long string).
          // Deferred a frame — iOS collapses a synchronous select() back to a caret
          // as it finishes installing focus/the keyboard.
          const input = e.target as HTMLInputElement;
          requestAnimationFrame(() => input.select());
        }}
        onBlur={(): void => {
          onTitleFocusChange(false);
          if (saveOnBlur) onTitleSave();
        }}
        onKeyDown={(event): void => {
          // Desktop retains its compact field convention. Mobile Enter belongs
          // to the system keyboard and never submits the rename.
          if (
            saveOnBlur && event.key === "Enter" &&
            !isImeKeyEvent(event.nativeEvent)
          ) {
            (event.target as HTMLInputElement).blur();
          }
        }}
        fullWidth
        sx={{ mt: 1, mb: 0.5 }}
      />
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
      <Stack spacing={0.65} sx={{ pt: 0.75, pb: 1 }}>
        <Stack
          direction="row"
          alignItems="baseline"
          justifyContent="space-between"
          spacing={2}
        >
          <Typography variant="caption" color="text.secondary">
            Context
          </Typography>
          <Typography variant="body2" sx={{ fontWeight: 600 }}>
            {hasContext
              ? `${Math.round(contextPercent)}% used`
              : "Waiting for usage"}
          </Typography>
        </Stack>
        <LinearProgress
          variant="determinate"
          value={contextPercent}
          color={contextPercent >= 90
            ? "error"
            : contextPercent >= 75
            ? "warning"
            : "primary"}
          aria-label="session context usage"
          sx={{
            height: 7,
            borderRadius: 99,
            bgcolor: "action.selected",
            "& .MuiLinearProgress-bar": { borderRadius: 99 },
          }}
        />
        {hasContext && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ textAlign: "right" }}
          >
            {contextUsed.toLocaleString()} / {contextSize.toLocaleString()}{" "}
            tokens
          </Typography>
        )}
        {cacheProtectionVisible &&
          (cacheProtection || cacheProtectionUnavailable) && (
          <Stack spacing={0.35} alignItems="flex-start">
            <Tooltip
              title={cacheProtection?.state === "protected"
                ? `${cacheIntervalDetail}. Last verified ${
                  cacheProtection.last_verified_at_ms
                    ? new Date(cacheProtection.last_verified_at_ms)
                      .toLocaleString()
                    : "recently"
                }. Next attempt ${
                  cacheProtection.next_attempt_at_ms
                    ? new Date(cacheProtection.next_attempt_at_ms)
                      .toLocaleString()
                    : "is being scheduled"
                }. Real agent work always preempts it.`
                : cacheProtection?.state === "disabled"
                ? `Automatic ${sessionProviderName} cache protection is disabled for this session. Base interval is ${cacheBaseInterval}.`
                : cacheProtectionUnavailable
                ? "The owning Machine could not report cache status. Agent work is unaffected."
                : "This session is large enough for protection; Cowboy is waiting for a verified ≥90% cache hit before scheduling a keepalive."}
            >
              <Chip
                size="small"
                variant="outlined"
                color={cacheProtection?.state === "protected"
                  ? "success"
                  : cacheProtectionUnavailable
                  ? "warning"
                  : "default"}
                label={cacheProtection?.state === "protected"
                  ? `Cache protected · ${
                    compactCacheTokens(cacheProtection.protected_tokens)
                  }`
                  : cacheProtection?.state === "disabled"
                  ? `${cacheOptionName} off`
                  : cacheProtectionUnavailable
                  ? "Cache status unavailable"
                  : `${cacheOptionName} learning`}
                sx={{ alignSelf: "flex-start" }}
              />
            </Tooltip>
            {cacheProtection && !cacheProtectionUnavailable && (
              <Typography variant="caption" color="text.secondary">
                {cacheIntervalDetail} · adaptive learning
              </Typography>
            )}
          </Stack>
        )}
        <Box sx={{ pt: 0.5 }}>
          <ButtonBase
            aria-label={actionsExpanded
              ? "Collapse session actions"
              : "Expand session actions"}
            aria-expanded={actionsExpanded}
            aria-controls={actionsPanelId}
            onClick={(): void => {
              haptic();
              onActionsExpandedChange(!actionsExpanded);
            }}
            sx={{
              width: "100%",
              minHeight: 44,
              px: 0.5,
              borderRadius: 1.5,
              justifyContent: "space-between",
              color: "text.secondary",
              textAlign: "left",
              "&:active": { bgcolor: "action.hover" },
            }}
          >
            <Typography variant="body2" sx={{ fontWeight: 650 }}>
              Session actions
            </Typography>
            <ExpandMore
              fontSize="small"
              sx={{
                transform: actionsExpanded ? "rotate(180deg)" : "none",
                transition: (theme) => theme.transitions.create("transform"),
              }}
            />
          </ButtonBase>
          <Collapse id={actionsPanelId} in={actionsExpanded} unmountOnExit>
            <Stack spacing={1} sx={{ pt: 0.75 }}>
              <Button
                fullWidth
                variant="outlined"
                startIcon={<Refresh />}
                aria-label="reload session from session settings"
                onClick={(event): void => {
                  event.currentTarget.blur();
                  onReload();
                }}
                sx={{ minHeight: 44, textTransform: "none" }}
              >
                Reload runtime
              </Button>
              {compactAction && (
                <Button
                  fullWidth
                  variant="outlined"
                  startIcon={<Compress />}
                  disabled={compacting}
                  aria-label="compact conversation from session settings"
                  onClick={(event): void => {
                    event.currentTarget.blur();
                    onSessionAction(compactAction);
                  }}
                  sx={{ minHeight: 44, textTransform: "none" }}
                >
                  {compacting ? "Compacting…" : "Compact"}
                </Button>
              )}
              {clearAction && (
                <Button
                  fullWidth
                  variant="outlined"
                  color="error"
                  startIcon={<CleaningServices />}
                  aria-label="clear conversation from session settings"
                  onClick={(event): void => {
                    event.currentTarget.blur();
                    onSessionAction(clearAction);
                  }}
                  sx={{ minHeight: 44, textTransform: "none" }}
                >
                  Clear
                </Button>
              )}
            </Stack>
          </Collapse>
        </Box>
      </Stack>
    </>
  );
}

export function SessionProviderSection({
  session,
}: {
  session: SessionMeta;
}): React.JSX.Element {
  return <>
    <SessionAccountDetails session={session} />
    {(!session.provider_account_id || session.provider_account_id === "default") &&
      <ServiceSessionProviderSection session={session} />}
  </>;
}

function ServiceSessionProviderSection({ session }: { session: SessionMeta }): React.JSX.Element {
  const { catalog } = useProviderCatalog();
  const entry = currentProviderEntry(
    session.provider,
    session.provider_version,
    session.provider_generation_digest,
  );
  const auth = catalog?.authentications.find((status) =>
    status.provider_id === session.provider ||
    status.authentication_scope === entry?.authentication_scope
  );
  const required = entry?.manifest.authentication.required === true;
  const ready = !required || auth?.authentication_state === "ready";
  const needsAttention = sessionProviderNeedsAttention({
    catalogReady: catalog !== null,
    required,
    ready,
  });
  const [expanded, setExpanded] = useState(true);
  const [actionsOpen, setActionsOpen] = useState(needsAttention);
  useEffect(() => {
    if (needsAttention) setExpanded(true);
    setActionsOpen(needsAttention);
  }, [needsAttention, session.id]);
  const presentation = entry
    ? resolveProviderAuthenticationPresentation(entry.manifest.authentication)
    : "none";
  const facts = entry
    ? sessionProviderFacts({
      vendor: entry.manifest.display.vendor,
      version: entry.provider_version,
      ...(ready && auth?.account_label
        ? { accountLabel: auth.account_label }
        : {}),
    })
    : [];
  const manageLabel = sessionProviderManageLabel(
    presentation === "none" ? "none" : presentation,
  );
  const summary = sessionProviderSummary({
    displayName: providerName(
      session.provider,
      session.provider_version,
      session.provider_generation_digest,
    ),
    required,
    ready,
    ...(auth?.account_label ? { accountLabel: auth.account_label } : {}),
    presentation: presentation === "none" ? "none" : presentation,
  });
  const panelId = useId();
  return (
    <>
      <Divider />
      <Box sx={{ py: 0.75 }}>
        <ButtonBase
          aria-label={expanded
            ? "Collapse provider"
            : "Expand provider"}
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={(): void => {
            haptic();
            setExpanded((current) => !current);
          }}
          sx={{
            width: "100%",
            minHeight: 44,
            px: 0.5,
            borderRadius: 1.5,
            justifyContent: "space-between",
            color: "text.secondary",
            textAlign: "left",
            "&:active": { bgcolor: "action.hover" },
          }}
        >
          <Box sx={{ minWidth: 0, pr: 1 }}>
            <Typography variant="body2" sx={{ fontWeight: 650 }}>
              Provider
            </Typography>
            <Typography
              variant="caption"
              color={needsAttention ? "warning.main" : "text.secondary"}
              sx={{ display: "block", lineHeight: 1.35 }}
            >
              {summary}
            </Typography>
          </Box>
          <ExpandMore
            fontSize="small"
            sx={{
              flexShrink: 0,
              transform: expanded ? "rotate(180deg)" : "none",
              transition: (theme) => theme.transitions.create("transform"),
            }}
          />
        </ButtonBase>
        <Collapse id={panelId} in={expanded}>
          <Box sx={{ pt: 0.5 }}>
            {entry && (
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{
                  display: "block",
                  px: 0.5,
                  pb: 0.25,
                  lineHeight: 1.4,
                }}
              >
                {entry.manifest.display.summary}
              </Typography>
            )}
            {entry && (
              <List dense disablePadding>
                {facts.map((row) => (
                  <SheetDetailRow
                    key={row.label}
                    label={row.label}
                    value={row.value}
                    mono={row.mono === true}
                  />
                ))}
                {sessionProviderShowsUsage({
                  ready,
                  ...(entry.manifest.host.account_usage?.provider
                    ? {
                      accountUsageProvider:
                        entry.manifest.host.account_usage.provider,
                    }
                    : {}),
                }) && (
                  <SessionProviderUsage
                    provider={session.provider}
                    {...(session.provider_version === undefined
                      ? {}
                      : { providerVersion: session.provider_version })}
                    {...(session.provider_generation_digest === undefined
                      ? {}
                      : {
                        providerDigest: session.provider_generation_digest,
                      })}
                  />
                )}
              </List>
            )}
            {ready && (
              <ButtonBase
                aria-label={actionsOpen
                  ? `Collapse ${manageLabel.toLowerCase()}`
                  : `Expand ${manageLabel.toLowerCase()}`}
                aria-expanded={actionsOpen}
                onClick={(): void => {
                  haptic();
                  setActionsOpen((current) => !current);
                }}
                sx={{
                  width: "100%",
                  minHeight: 44,
                  px: 0.5,
                  mt: 0.25,
                  borderRadius: 1.5,
                  justifyContent: "space-between",
                  color: "text.secondary",
                  textAlign: "left",
                  "&:active": { bgcolor: "action.hover" },
                }}
              >
                <Typography variant="body2" sx={{ fontWeight: 650 }}>
                  {manageLabel}
                </Typography>
                <ExpandMore
                  fontSize="small"
                  sx={{
                    flexShrink: 0,
                    transform: actionsOpen ? "rotate(180deg)" : "none",
                    transition: (theme) =>
                      theme.transitions.create("transform"),
                  }}
                />
              </ButtonBase>
            )}
            <Collapse in={!ready || actionsOpen} unmountOnExit>
              <Box sx={{ pt: 0.5 }}>
                <SessionProviderAccess providerId={session.provider} />
              </Box>
            </Collapse>
          </Box>
        </Collapse>
      </Box>
    </>
  );
}

// Queue pause and Page view are infrequent. Keep them behind one disclosure
// so the sheet opens on identity, Provider, and Agent. Do not call this
// Workspace — that word already means a project checkout. A paused queue
// still names itself on the collapsed row.
function WorkspaceOptionsSection({
  session,
  expanded,
  onExpandedChange,
  projection,
  onProjectionChange,
}: {
  session: SessionMeta;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  projection?: TranscriptProjection | undefined;
  onProjectionChange?: ((projection: TranscriptProjection) => void) | undefined;
}): React.JSX.Element {
  const paused = session.paused ?? false;
  const pageView = projection === "explore";
  const panelId = useId();
  return (
    <>
      <Divider />
      <Box sx={{ py: 0.75 }}>
        <ButtonBase
          aria-label={expanded
            ? "Collapse queue and view"
            : "Expand queue and view"}
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={(): void => {
            haptic();
            onExpandedChange(!expanded);
          }}
          sx={{
            width: "100%",
            minHeight: 44,
            px: 0.5,
            borderRadius: 1.5,
            justifyContent: "space-between",
            color: paused ? "warning.main" : "text.secondary",
            textAlign: "left",
            "&:active": { bgcolor: "action.hover" },
          }}
        >
          <Box sx={{ minWidth: 0, pr: 1 }}>
            <Typography variant="body2" sx={{ fontWeight: 650 }}>
              Queue & view
            </Typography>
            <Typography
              variant="caption"
              color={paused ? "warning.main" : "text.secondary"}
              sx={{ display: "block", lineHeight: 1.35 }}
            >
              {workspaceOptionsSummary({ queuePaused: paused, pageView })}
            </Typography>
          </Box>
          <ExpandMore
            fontSize="small"
            sx={{
              flexShrink: 0,
              transform: expanded ? "rotate(180deg)" : "none",
              transition: (theme) => theme.transitions.create("transform"),
            }}
          />
        </ButtonBase>
        <Collapse id={panelId} in={expanded} unmountOnExit>
          <Stack spacing={0.5} sx={{ pt: 0.5 }}>
            <QueueDeliverySettings sessionId={session.id} pauseControl={false} />
            <Stack
              direction="row"
              alignItems="center"
              spacing={2}
              sx={{ minHeight: 56 }}
            >
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Typography variant="body2">Pause queue</Typography>
                <Typography variant="caption" color="text.secondary">
                  Hold the queue: a running turn still finishes, but queued and
                  scheduled sends wait until you resume.
                </Typography>
              </Box>
              <Switch
                edge="end"
                checked={paused}
                color="warning"
                inputProps={{ "aria-label": "pause queue" }}
                onChange={(e): void => {
                  haptic();
                  setPaused(session.id, e.target.checked);
                }}
              />
            </Stack>
            {projection && onProjectionChange && (
              <Stack
                direction="row"
                alignItems="center"
                spacing={2}
                sx={{ minHeight: 56 }}
              >
                <Box sx={{ flex: 1, minWidth: 0 }}>
                  <Typography variant="body2" sx={{ fontWeight: 650 }}>
                    Page view
                  </Typography>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ display: "block", lineHeight: 1.35 }}
                  >
                    One prompt and response per page
                  </Typography>
                </Box>
                <Switch
                  inputProps={{ "aria-label": "Page view" }}
                  checked={pageView}
                  onChange={(_event, checked): void =>
                    onProjectionChange(checked ? "explore" : "history")}
                />
              </Stack>
            )}
          </Stack>
        </Collapse>
      </Box>
    </>
  );
}

function SessionProviderUsage({
  provider,
  providerVersion,
  providerDigest,
}: {
  provider: string;
  providerVersion?: string;
  providerDigest?: string;
}): React.JSX.Element {
  // Controller-owned usage shared by every surface and device.
  const snapshot = useStoreSelector((state) => state.usage);
  const [error, setError] = useState<string | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  useEffect(() => {
    const timer = globalThis.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => globalThis.clearInterval(timer);
  }, []);
  useEffect(() => setClock(Date.now()), [snapshot]);
  const account = providerUsageAccount(provider, providerVersion, providerDigest);
  const refreshing = account !== undefined && usageRefreshing(snapshot, account);
  const load = useCallback(async (manual: boolean): Promise<void> => {
    if (manual) {
      if (!account) {
        throw new Error("Usage is unavailable for this session's Provider.");
      }
      await requestUsageRefresh(account);
    } else {
      await loadUsageSnapshot();
    }
    setError(null);
  }, [account]);
  useEffect(() => {
    const ctrl = new AbortController();
    void loadUsageSnapshot(ctrl.signal).catch((cause: unknown) => {
      if (ctrl.signal.aborted) return;
      setError(cause instanceof Error ? cause.message : "Could not load usage");
    });
    return (): void => ctrl.abort();
  }, [provider]);
  const usage = providerUsage(
    snapshot,
    provider,
    providerVersion,
    providerDigest,
  );
  const rows = sessionProviderUsageRows(usage, clock);
  const refresh = (
    <NetworkIconButton
      aria-label="Refresh provider usage"
      size="small"
      reliableTouch
      networkAction={() => load(true)}
      disabled={refreshing}
      sx={{
        width: "2.75em",
        height: "2.75em",
        p: 0,
        color: "text.secondary",
        "& .MuiSvgIcon-root": { fontSize: "1em" },
      }}
    >
      <Refresh />
    </NetworkIconButton>
  );
  if (error) {
    return <SheetDetailRow label="Usage" value={error} action={refresh} />;
  }
  if (!snapshot) {
    return <SheetDetailRow label="Usage" value="Loading…" />;
  }
  if (rows.length === 0) {
    return (
      <SheetDetailRow
        label="Usage"
        value={sessionProviderUsageEmptyMessage(usage)}
        action={refresh}
      />
    );
  }
  return (
    <>
      {rows.map((row) => (
        <Box key={row.id}>
          <SheetDetailRow
            label={row.label}
            value={row.value}
            action={row.id === "usage-updated" ? refresh : undefined}
          />
          {row.remaining !== undefined && (
            <LinearProgress
              variant="determinate"
              value={row.remaining}
              aria-label={`${row.label} remaining`}
              sx={{
                height: 5,
                mb: 0.75,
                borderRadius: 99,
                bgcolor: "action.selected",
                "& .MuiLinearProgress-bar": { borderRadius: 99 },
              }}
            />
          )}
        </Box>
      ))}
    </>
  );
}

function SheetDetailRow({
  label,
  value,
  mono = false,
  action,
}: {
  label: string;
  value: string;
  mono?: boolean;
  action?: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        py: 0.75,
        display: "flex",
        gap: 2,
        alignItems: action ? "center" : "baseline",
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
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </Typography>
      {action}
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
