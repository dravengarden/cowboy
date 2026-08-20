import {
  lazy,
  memo,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  Suspense,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  ClickAwayListener,
  Collapse,
  DialogContentText,
  Divider,
  IconButton,
  keyframes,
  LinearProgress,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Paper,
  Popper,
  Snackbar,
  Stack,
  Switch,
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
  Check,
  ChevronRight,
  CleaningServices,
  Close,
  CloudUpload,
  CloseFullscreen,
  Compress,
  DeleteOutline,
  DragIndicator,
  DriveFileMoveOutlined,
  EditNoteOutlined,
  EditOutlined,
  ExpandMore,
  InsertDriveFileOutlined,
  KeyboardHide,
  MoreVert,
  OpenInFull,
  Refresh,
  Schedule,
  Send,
  Stop,
  SwapVert,
  Tune,
  Undo,
  VerticalAlignBottom,
  VerticalAlignTop,
  Visibility,
} from "@mui/icons-material";
import {
  type ComposerEditorHandle,
  type ComposerEditorSelection,
  PlatformComposerEditor,
} from "./composer/PlatformComposerEditor";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { useComposerDraftController } from "./composer/useComposerDraftController";
import {
  type NativeClipboardImagePasteRequest,
  runNativeClipboardImagePaste,
} from "./composer/nativeClipboardImagePaste";
import {
  didMobileSoftwareKeyboardClose,
  dismissMobileSoftwareKeyboard,
  mobilePendingKeyboardCloseSettleMs,
  beginMobileEditorFocusTransfer,
  isMobileEditorFocusTransferPending,
  releaseMobileComposerFocus,
  shouldPresentMobileKeyboardSurface,
  noteMobileKeyboardDismissed,
  clearMobileKeyboardDismissed,
} from "./composer/mobileComposerFocus";
import { useKeyboardOpen } from "./keyboardInset";
import { attachmentTrayForSurface } from "./composer/attachmentPresentation";
import {
  canReturnFromPendingRow,
  homeForOrigin,
  pendingSyncAppearance,
  returnLabelForHome,
} from "./localFirstDelivery";
import type { ComposerWorkspaceProps } from "./composer/contracts";
import { resolveSessionAction, type SessionAction } from "./agentCommands";
import {
  currentProviderEntry,
  useProviderCatalog,
} from "./providerCatalog";
import { providerName } from "./providerPresentation";
import { SessionProviderAccess } from "./ProviderManagement";
import { resolveProviderAuthenticationPresentation } from "../../packages/provider-ui-sdk/src/index.ts";
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
  type UsageSnapshot,
} from "./usageLimits";
import { SessionReloadDialog } from "./SessionReloadDialog";
import { createPortal, flushSync } from "react-dom";
import { FullscreenComposer } from "./FullscreenComposer";
import { ComposerToolbarSettings } from "./ComposerToolbarSettings";
import { useComposerToolbar } from "./composerToolbarConfig";
import { MobileComposerFormatActions } from "./MobileComposerFormatActions";
import {
  MobileComposerAccessoryButton,
  MobileComposerAccessoryDock,
  MobileComposerEditingBar,
  MobileComposerFixedActionSlot,
} from "./MobileComposerAccessoryDock";
import { MessagePreview } from "./MessagePreview";
import { useTouchComposer } from "./ComposerTextarea";
import { shouldExpandInlineComposer } from "./composer/mobileCompactEditorPolicy";
import {
  pendingEditLiveText,
  pendingPanelDisclosureDecision,
  pendingRowVisibleText,
} from "./pendingEditLifecycle";
import { isImeKeyEvent } from "./imeKey";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ALT_LABEL, ENTER_LABEL, MOD_LABEL } from "./platform";
import { ShortcutKeycap } from "./ShortcutKeycap";
import { openLightbox } from "./ResourceLightbox";
import { PlanDock } from "./PlanDock";
import {
  mobileComposerFocusMotion,
  mobileComposerIdleEditorMinHeight,
  mobileComposerKeyboardGap,
  mobileComposerPanelFrameSx,
  mobileComposerPanelHeaderMinHeight,
  mobileComposerStackGap,
  mobilePendingRowMinHeight,
} from "./mobileComposerPrimitives";
import {
  mobileFocusedComposerFill,
  mobileFocusedComposerSurfaceSx,
} from "./mobileComposerSurface";
import { TurnStatusOverlay } from "./TurnStatusOverlay";
import { PermissionOverlay } from "./PermissionOverlay";
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
import {
  desktopListItemSx,
  desktopSessionActionSx,
  desktopSurfaceSx,
} from "./desktop/DesktopEmbeddedControl";
import {
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
} from "./desktop/commands/workspaceShortcuts";
import { listJumpKey } from "./desktop/commands/listNavigation";
import { shortcutAvailability } from "./desktop/commands/shortcutAvailability";
import {
  type Attachment,
  filesToAttachments,
  fileToAttachment,
  pendingImageAttachment,
  promoteUnplacedImageTokens,
  reconcileDeletedInlineImages,
  settlePendingAttachments,
  stripImageTokens,
} from "./attachments";
import {
  getInlineAttachment,
  registerInlineAttachment,
  seedInlineAttachments,
  setImageTapHandler,
} from "./inlineImages";

import {
  activateAllDrafts,
  activateDraft,
  clearDrafts,
  clearQueue,
  discardQueued,
  returnFailedQueued,
  editDraft,
  editQueued,
  forcePushQueued,
  moveDraft,
  openSession,
  type QueuedMessage,
  queuedToDraft,
  removeDraft,
  removeQueued,
  renameSession,
  reorderDrafts,
  reorderQueue,
  requestSendQueued,
  resetSession,
  retryQueued,
  scheduleDraft,
  send,
  setPaused,
  setQueueEditing,
  submitPrompt,
  unscheduleDraft,
  useConnected,
  useStoreSelector,
} from "./store";
import { setActiveSessionId } from "./controlPlane";
import { ScheduleSheet } from "./ScheduleSheet";
import { fireLabel } from "./scheduleTime";
import { haptic, importantHaptic, navigationHaptic } from "./haptic";
import { useSortable } from "./useSortable";
import { useNavbarAtBottom } from "./navbarSettings";
import { useReadingSettings } from "./readingSettings";
import { useReliableTouchTap } from "./useReliableTouchTap";
import {
  NetworkButton,
  NetworkIconButton,
  useNetworkActionState,
} from "./NetworkActionFeedback";
import { originLabel } from "./protocol";
import {
  providerConfigOptionDisabled,
  providerConfigOptionOrder,
  providerConfigOptionPresentations,
  providerConfigOptions,
  providerConfigSurfaceDisabled,
} from "./providerConfigOptions";
import {
  activeRunConfigPreset,
  type RunConfigPreset,
  runConfigPresetChanges,
  runConfigPresets,
} from "./runConfigPresets";
import {
  DEEPSEEK_CACHE_BASE_INTERVAL_LABEL,
  DEEPSEEK_CACHE_MIN_HIT_TOKENS,
} from "./deepseekUsage";
import type {
  AvailableCommand,
  ConfigOption,
  Delivery,
  DraftSchedule,
  SessionMeta,
  Status,
} from "./protocol";
import { sessionProjectLabel } from "./sessionProject";
import { ConfirmSheet, Sheet } from "./Sheet";
import { MobileSheetDismiss } from "./_shell";
import {
  OPEN_SESSION_SETTINGS_EVENT,
  type SessionSettingsFocus,
  sessionSettingsFocusFromEvent,
} from "./sessionSettingsOpen";
import { useStore as usePrefStore } from "./_store/mod.ts";
import {
  PENDING_ARRIVAL_FLASH_MS,
  type PendingArrival,
  pendingRowMatchesArrival,
  scrollPendingRowIntoView,
  subscribePendingArrival,
} from "./pendingPanelState";
import {
  collapseComposerStackPanel,
  composerStackExpandedStore,
  toggleComposerStackPanel,
} from "./composerStackAccordion";

const DesktopContextShortcut = lazy(async () => {
  const module = await import("./desktop/commands/DesktopContextShortcut");
  return { default: module.DesktopContextShortcut };
});
const DesktopComposerCommandBindings = lazy(async () => {
  const module = await import("./desktop/commands/DesktopComposerShortcuts");
  return { default: module.DesktopComposerCommandBindings };
});
const DesktopPendingEditCommandBindings = lazy(async () => {
  const module = await import("./desktop/commands/DesktopPendingEditShortcuts");
  return { default: module.DesktopPendingEditCommandBindings };
});
const DesktopRegionShortcut = lazy(async () => {
  const module = await import("./desktop/DesktopRegionShortcut");
  return { default: module.DesktopRegionShortcut };
});
const DesktopListJumpKeycap = lazy(async () => {
  const module = await import("./desktop/commands/DesktopListJumpKeycap");
  return { default: module.DesktopListJumpKeycap };
});

const EMPTY_CONFIG_OPTIONS: ConfigOption[] = [];
const EMPTY_QUEUED_MESSAGES: QueuedMessage[] = [];



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
          <DialogContentText sx={{ mt: 1.5, fontSize: "0.8125rem" }}>
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
          </DialogContentText>
        </>
      )}
    </ConfirmSheet>
  );
}

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
    onOpenConfig,
    onAttach,
    attachments = [],
    onRemoveAttachment,
    onSaveDraft,
    onCollapse,
    onExpand,
    onForcePush,
    onJumpFront,
    submitLabel = "Send",
    submitIcon,
    desktop = false,
  }: {
    readonly dead: boolean;
    readonly sendable: boolean;
    readonly onTrigger: (trigger: string) => void;
    readonly onSend: () => void;
    readonly options?: ConfigOption[];
    readonly showSkeleton?: boolean;
    /** Open the config popup (the ⚙ button) — the labeled Mode/Model/Effort
     *  dropdowns live in that sheet, not inline on the bar. */
    readonly onOpenConfig?: (() => void) | undefined;
    readonly onAttach?: (() => void) | undefined;
    readonly attachments?: Attachment[];
    readonly onRemoveAttachment?: ((id: string) => void) | undefined;
    readonly onSaveDraft?: (() => void) | undefined;
    readonly onCollapse?: (() => void) | undefined;
    /** Expand the inline editor to the fullscreen sheet (↗) — mirror of onCollapse. */
    readonly onExpand?: (() => void) | undefined;
    /** Receives the ⋮ button so the caller can anchor its force-push confirm. */
    readonly onForcePush?: ((anchor: HTMLElement) => void) | undefined;
    /** "Jump to front of queue" (no interrupt) — provided only when there's a
     *  queue to jump ahead of. */
    readonly onJumpFront?: (() => void) | undefined;
    /** Semantic label/icon for the primary commit action. Row editors are
     *  live-saved, so their action is Done rather than Send. */
    readonly submitLabel?: string;
    readonly submitIcon?: React.ReactNode;
    /** Desktop-only floating shortcut hints. Mobile keeps the touch toolbar. */
    readonly desktop?: boolean;
  },
): React.JSX.Element {
  // The ⚙ opens the config POPUP (the labeled Mode/Model/Effort dropdowns live in
  // that sheet — see ComposerSheet — not inline on the bar).
  const hasConfig = showSkeleton || options.length > 0;
  // Secondary actions (Save-draft / Jump-to-front / Force-push) sit inline on a
  // roomy (≥ sm) bar, but fold into a ⋮ overflow on the narrow phone tier — so the
  // PRIMARY buttons keep their full, app-consistent tap target instead of shrinking
  // to cram the whole set onto one row (and Exit-fullscreen never orphans below).
  const theme = useTheme();
  const roomy = useMediaQuery(theme.breakpoints.up("sm"));
  const [moreMenu, setMoreMenu] = useState<HTMLElement | null>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const hasSecondary = Boolean(onSaveDraft || onJumpFront || onForcePush);
  const desktopShortcut = (
    child: ReactNode,
    badge: string,
    shortcut: string,
    enabled = true,
  ): ReactNode =>
    desktop
      ? (
        <Suspense fallback={child}>
          <DesktopContextShortcut
            badge={badge}
            shortcut={shortcut}
            enabled={enabled}
          >
            {child}
          </DesktopContextShortcut>
        </Suspense>
      )
      : child;
  return (
    <Stack
      direction="column"
      spacing={0.75}
      sx={{
        width: "100%",
        pt: 1,
        borderTop: (t) => `1px solid ${t.palette.divider}`,
      }}
    >
      {
        /* IMAGES render inline in the editor (Obsidian-style); the docked tray now
          carries only NON-image files (code, etc.), which have no inline form. */
      }
      {attachments.some((a) => !a.isImage) && onRemoveAttachment && (
        <AttachmentPreviews
          attachments={attachments.filter((a) => !a.isImage)}
          onRemove={onRemoveAttachment}
        />
      )}
      {
        /* LEFT-aligned with a fixed gap (not space-evenly — a few icons shouldn't
          stretch across the whole width; reads cleaner next to the attachment
          thumbnail), wrapping to a second row when they don't all fit one line. No
          flex spacer / overflow-scroll / breakout, so nothing clips or pushes off. */
      }
      <Stack
        direction="row"
        alignItems="center"
        // Left-aligned with a comfortable gap. Buttons keep their FULL, default tap
        // target (no width/height shrink) so the bar matches every other icon button
        // in the app — usability over cramming. Overflow is handled by folding the
        // secondary actions into the ⋮ (see below), not by shrinking. Still wraps as
        // a last resort on the very narrowest widths.
        sx={{
          justifyContent: "flex-start",
          flexWrap: "wrap",
          gap: 0.25,
          "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
        }}
      >
        <Tooltip title="Slash command / skill">
          <span>
            {desktopShortcut(
              <IconButton
                aria-label="slash command"
                disabled={dead}
                sx={TOOLBAR_ICON_BTN}
                onClick={(): void => onTrigger("/")}
              >
                <Box
                  component="span"
                  sx={{ fontSize: "1.25rem", fontWeight: 700, lineHeight: 1 }}
                >
                  /
                </Box>
              </IconButton>,
              `${ALT_LABEL}/`,
              `${ALT_LABEL}/ · slash command`,
              !dead,
            )}
          </span>
        </Tooltip>
        <Tooltip title="Reference a file (@)">
          <span>
            {desktopShortcut(
              <IconButton
                aria-label="reference a file"
                disabled={dead}
                sx={TOOLBAR_ICON_BTN}
                onClick={(): void => onTrigger("@")}
              >
                <AlternateEmail />
              </IconButton>,
              `${ALT_LABEL}R`,
              `${ALT_LABEL}R · reference a file`,
              !dead,
            )}
          </span>
        </Tooltip>
        {onAttach && (
          <Tooltip title="Attach image or file">
            <span>
              {desktopShortcut(
                <IconButton
                  aria-label="attach image or file"
                  disabled={dead}
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onAttach}
                >
                  <AttachFile />
                </IconButton>,
                `${ALT_LABEL}A`,
                `${ALT_LABEL}A · attach file`,
                !dead,
              )}
            </span>
          </Tooltip>
        )}
        {hasConfig && onOpenConfig && (
          <Tooltip title="Options">
            <span>
              <IconButton
                aria-label="options"
                disabled={dead}
                sx={TOOLBAR_ICON_BTN}
                onClick={onOpenConfig}
              >
                <Tune />
              </IconButton>
            </span>
          </Tooltip>
        )}
        {
          /* Secondary actions: inline on a roomy (≥ sm) bar, folded into the ⋮ on the
            narrow phone tier so the primary buttons keep their full tap size. Force
            push only while busy/starting; jump-to-front only when there's a queue. */
        }
        {hasSecondary &&
          (roomy
            ? (
              <>
                {onJumpFront && (
                  <Tooltip title="Jump to front of queue">
                    <span>
                      <IconButton
                        aria-label="jump to front of queue"
                        sx={TOOLBAR_ICON_BTN}
                        onClick={onJumpFront}
                      >
                        <VerticalAlignTop />
                      </IconButton>
                    </span>
                  </Tooltip>
                )}
                {onForcePush && (
                  <Tooltip title="Force push">
                    <span>
                      <IconButton
                        color="warning"
                        aria-label="force push"
                        disabled={!sendable}
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
              </>
            )
            : (
              <Tooltip title="More actions">
                <IconButton
                  ref={moreRef}
                  aria-label="more actions"
                  sx={TOOLBAR_ICON_BTN}
                  onClick={(e): void => setMoreMenu(e.currentTarget)}
                >
                  <MoreVert />
                </IconButton>
              </Tooltip>
            ))}
        <Tooltip title={submitLabel}>
          <span>
            {desktopShortcut(
              <IconButton
                color="primary"
                aria-label={submitLabel.toLowerCase()}
                disabled={!sendable}
                sx={TOOLBAR_ICON_BTN}
                onClick={onSend}
              >
                {submitIcon ?? <Send />}
              </IconButton>,
              `${MOD_LABEL}↵`,
              `${MOD_LABEL}Enter · ${submitLabel}`,
              sendable,
            )}
          </span>
        </Tooltip>
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
        {onExpand && (
          <Tooltip title="Expand editor">
            <span>
              {desktopShortcut(
                <IconButton
                  aria-label="expand editor"
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onExpand}
                >
                  <OpenInFull />
                </IconButton>,
                `${ALT_LABEL}E`,
                `${ALT_LABEL}E · expand editor`,
              )}
            </span>
          </Tooltip>
        )}
      </Stack>
      {
        /* Narrow-tier overflow for the secondary actions (folded out of the row
          above when !roomy). Force-push anchors its confirm popover to the ⋮ button
          (moreRef) since the menu item itself unmounts on select. */
      }
      <Menu
        anchorEl={moreMenu}
        open={moreMenu !== null}
        onClose={(): void => setMoreMenu(null)}
        anchorOrigin={{ vertical: "top", horizontal: "right" }}
        transformOrigin={{ vertical: "bottom", horizontal: "right" }}
      >
        {onSaveDraft && (
          <MenuItem
            disabled={!sendable}
            onClick={(): void => {
              setMoreMenu(null);
              onSaveDraft();
            }}
          >
            <EditNoteOutlined fontSize="small" sx={{ mr: 1 }} />
            Save as draft
          </MenuItem>
        )}
        {onJumpFront && (
          <MenuItem
            onClick={(): void => {
              setMoreMenu(null);
              onJumpFront();
            }}
          >
            <VerticalAlignTop fontSize="small" sx={{ mr: 1 }} />
            Jump to front of queue
          </MenuItem>
        )}
        {onForcePush && (
          <MenuItem
            disabled={!sendable}
            onClick={(): void => {
              setMoreMenu(null);
              if (moreRef.current) onForcePush(moreRef.current);
            }}
          >
            <Bolt fontSize="small" color="warning" sx={{ mr: 1 }} />
            Force push
          </MenuItem>
        )}
      </Menu>
    </Stack>
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
  const desktopShortcut = (
    child: ReactNode,
    badge: string,
    shortcut: string,
    enabled = true,
  ): ReactNode =>
    desktop
      ? (
        <Suspense fallback={child}>
          <DesktopContextShortcut
            badge={badge}
            shortcut={shortcut}
            enabled={enabled}
          >
            {child}
          </DesktopContextShortcut>
        </Suspense>
      )
      : child;
  const editorRef = useRef<ComposerEditorHandle>(null);
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
    forceTracked,
    jumpToFrontTracked: jumpCurrentPromptToFrontTracked,
    saveAsDraftTracked,
    scheduleNew,
  } = useComposerDraftController(sessionId, editorRef, {
    // Desktop CodeMirror owns its document. Keeping its hot path in refs avoids
    // rerendering the entire Prompt pane for every IME composition update;
    // touch-native editors still require the live React value.
    mirrorTextInReact: surface !== "desktop",
  });
  // Clearing is independent from sending. A still-uploading attachment cannot
  // be sent yet, but it is still user-owned staged content and must remain
  // removable from the utility rail.
  const clearable = text.trim().length > 0 || attachments.length > 0;
  const submitAndNotify = useCallback((): boolean => {
    if (preparing) return false;
    const submitted = submit();
    if (submitted) {
      if (!desktop) dismissMobileSoftwareKeyboard();
      onSubmitted?.();
    }
    return submitted;
  }, [desktop, onSubmitted, preparing, submit]);
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
    if (preparing) return;
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
    preparing,
    submitFeedback,
    submitTracked,
  ]);
  const sendTap = useReliableTouchTap<HTMLButtonElement>(() =>
    submitWithFeedback()
  );
  const fileInputRef = useRef<HTMLInputElement>(null);
  const draftList = useStoreSelector((snapshot) =>
    snapshot.drafts.get(sessionId) ?? EMPTY_QUEUED_MESSAGES
  );
  const queue = useStoreSelector((snapshot) =>
    snapshot.queues.get(sessionId) ?? EMPTY_QUEUED_MESSAGES
  );
  const sessionState = useStoreSelector(
    (snapshot) => composerSessionSlice(snapshot.sessions, sessionId),
    sameComposerSessionSlice,
  );
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
  // iOS can omit the compatibility click after a touch that also preserves
  // the focused editor. Commit stationary image actions on pointerup while
  // retaining ordinary mouse, keyboard, and assistive click activation.
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
  // Long-press-send → force-push: hold the Queue button ~450ms to pop a confirm
  // that interrupts the running turn and runs this prompt next (skipping the
  // queue). `holding` drives the fill ring; `forceAnchor` anchors the non-modal
  // confirmation without ending the iOS editor focus session.
  const [holding, setHolding] = useState(false);
  const [forceAnchor, setForceAnchor] = useState<HTMLElement | null>(null);
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
  useEffect(() => {
    if (forceAnchor === null) return undefined;
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape" || isImeKeyEvent(event)) return;
      event.preventDefault();
      event.stopPropagation();
      setForceAnchor(null);
    };
    globalThis.addEventListener("keydown", closeOnEscape, true);
    return (): void =>
      globalThis.removeEventListener("keydown", closeOnEscape, true);
  }, [forceAnchor]);
  const [desktopMoreAnchor, setDesktopMoreAnchor] = useState<
    HTMLElement | null
  >(null);
  const desktopMoreButtonRef = useRef<HTMLButtonElement | null>(null);
  const desktopToolbarRef = useRef<HTMLDivElement | null>(null);
  // The split Prompt pane is user-resizable, so viewport breakpoints cannot tell
  // us whether its action row has room. Measure the row itself and expose every
  // delivery action whenever it can fit; fold only below the real content width.
  const [desktopActionsExpanded, setDesktopActionsExpanded] = useState(!column);
  useEffect(() => {
    if (!desktop) return undefined;
    const el = desktopToolbarRef.current;
    if (!el) return undefined;
    const update = (): void => {
      const expanded = el.getBoundingClientRect().width >= 540;
      setDesktopActionsExpanded(expanded);
      if (expanded) setDesktopMoreAnchor(null);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return (): void => observer.disconnect();
  }, [desktop]);
  // The Queue button — also the anchor for a KEYBOARD-triggered force-push (held
  // ⌘⏎), so the confirm rises from the same spot whether opened by hold or key.
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
  const starting = status === "starting";
  // A compaction is running right now: the turn is busy AND the live tail is
  // Claude Code's "Compacting..." notice (covers both a hand-fired /compact and
  // the agent's own auto-compaction). Drives the Compact button's disabled +
  // spinner state so a second /compact can't be fired mid-run.
  const compacting = busy && timelineState.compactingTail;
  // Queue manually paused: keep the ⚡ force button usable so a message can still
  // be pushed PAST the held queue (run now) even while the agent is idle.
  const paused = sessionState.paused;
  // Interrupted is a dead/resumable state too (a turn cut off by a daemon
  // restart) — the composer treats it like exited/crashed: "send to resume".
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
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
      return submitPrompt(sessionId, a.command, []);
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
    // Keep the native textarea as iOS's first responder until the tap/hold is
    // resolved. Blurring it here starts keyboard dismissal and can delay or
    // swallow the trailing click after the Composer reflows.
    e.preventDefault();
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
    submitWithFeedback();
  }
  async function confirmForce(): Promise<void> {
    const confirmation = forceTracked();
    if (confirmation === null) return;
    await confirmation;
    setForceAnchor(null);
    dismissAfterMobileDelivery();
    setComposeFs(false);
  }
  // "Jump to front of queue" (no interrupt): send the composed prompt to the
  // FRONT of the queue so it runs next after the current turn, ahead of the rest
  // of the queue. Only meaningful when there's already a queue to jump ahead of.
  function jumpToFront(): void {
    const confirmation = jumpCurrentPromptToFrontTracked(queue.length);
    if (confirmation === null) return;
    void confirmation.then(dismissAfterMobileDelivery).catch(() => undefined);
  }
  // Enter confirms the force-push popover (it doesn't autofocus a button the way
  // the Dialogs do). Held-⌘⏎ repeats are ignored inside the hook, so the still-
  // down Enter that opened it can't self-confirm — a fresh press does.
  useConfirmEnter(forceAnchor !== null, () => {
    void confirmForce();
  });
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
      scheduleDraft(sessionId, { id: scheduleTarget.id, fireAtMs, delivery });
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
        pt: desktop ? 1 : "var(--mobile-composer-boundary-gap)",
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
          pt: 1,
          pb: 1,
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
              paddingTop: "10px",
              paddingBottom: "10px",
            },
          "& [data-desktop-region='prompt.queued'][data-desktop-focused='true'] [data-desktop-aux-list], & [data-desktop-region='prompt.draft'][data-desktop-focused='true'] [data-desktop-aux-list]":
            {
              paddingBottom: "4px",
            },
        }),
      }}
    >
      {
        /* Status is the first ordinary stack slot, not a second absolute layer.
          A pending tool-permission outranks the turn-status pill (a blocking
          decision beats a status), and the two remain mutually exclusive. */
      }
      {pendingPermission
        ? (
          <PermissionOverlay
            item={pendingPermission}
            sessionId={sessionId}
            {...(desktop
              ? {
                shortcutForAction: (action: "approve" | "reject") => (
                  <Suspense fallback={null}>
                    <DesktopRegionShortcut
                      shortcut={action === "approve" ? "A" : "R"}
                      title={action === "approve"
                        ? "Allow once"
                        : "Reject once"}
                      showWhenPane="conversation"
                    />
                  </Suspense>
                ),
              }
              : {})}
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
                    singleKeycap={`${MOD_LABEL}P`}
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
          if (scheduleTarget?.id !== undefined) {unscheduleDraft(
              sessionId,
              scheduleTarget.id,
            );}
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
        /* The composer CARD (Zed-style): one outlined Paper owning the box — the
          editor sits borderless inside, the Send/Queue + Stop + ⋮ kebab overlay
          its bottom-right (`endInset` reserves text room). Transparent fill so it
          floats over the frosted bottom slab (a solid paper would hide the glass).
          A flex column so a later step can pin an inline toolbar to the bottom. */
      }
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
        // Column mode is a dedicated writing workspace. Its subtle card boundary
        // makes an empty tall editor read as an intentional canvas, not a blank
        // hole between the session rail and transcript.
        variant="outlined"
        elevation={0}
        sx={{
          position: "relative",
          display: !desktop && mobilePendingEditing ? "none" : "flex",
          flexDirection: "column",
          ...(surface === "desktop" && {
            ...desktopSurfaceSx({ interactive: false, focusWithin: true }),
          }),
          bgcolor: column
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
                borderTopColor: (t) => alpha(t.palette.divider, 0.42),
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
              endInset={36}
              // Hold ⌘⏎ while busy → the same force-push confirm the Queue button's
              // long-press opens, anchored to that button.
              holdToForce={!preparing && (busy || starting)}
              onForceHold={(): void => {
                if (!sendable || queueBtnRef.current === null) return;
                haptic();
                setForceAnchor(queueBtnRef.current);
              }}
              sessionId={sessionId}
              commands={(): AvailableCommand[] => availableCommands}
              placeholder={preparing
                ? "You can start typing while this session prepares…"
                : dead
                ? "Send to resume this session…"
                : "Message the agent…"}
              // Vim is desktop-only — never load it on touch (no physical keyboard /
              // modal editing). ComposerEditor also gates the actual vim import on a
              // fine pointer, so this is belt-and-suspenders.
              vim={touchInput ? false : vim}
              onVimMode={setVimMode}
              onPasteFiles={(files): void =>
                addFiles(files, { preserveFocus: true })}
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
          /* Inline bottom toolbar INSIDE the card (Zed layout): the / @ 📎 triggers
            + config on the left, a flex spacer, then the send/queue/stop + ⋮ action
            cluster pinned to the card's right edge. This replaces BOTH the old
            separate toolbar strip below the input AND the absolute send overlay —
            one cohesive card. `px`/`pb` (not the nav gutters) inset the row to the
            card's own edges. */
        }
        {desktop
          ? (
            <>
              <Suspense fallback={null}>
                <DesktopComposerCommandBindings
                  sendable={sendable}
                  canAttach={!dead}
                  canJumpFront={queue.length > 0}
                  canForce={busy || starting || paused}
                  canMore={!desktopActionsExpanded}
                  onSlash={(): void => editorRef.current?.insertTrigger("/")}
                  onReference={(): void =>
                    editorRef.current?.insertTrigger("@")}
                  onAttach={(): void => fileInputRef.current?.click()}
                  onSaveDraft={saveDraft}
                  onSchedule={(): void =>
                    setScheduleTarget({ id: undefined, initial: null })}
                  onJumpFront={jumpToFront}
                  onForce={(): void => {
                    if (queueBtnRef.current) {
                      setForceAnchor(queueBtnRef.current);
                    }
                  }}
                  onMore={(): void =>
                    setDesktopMoreAnchor(desktopMoreButtonRef.current)}
                />
              </Suspense>
              <Stack
                ref={desktopToolbarRef}
                direction="row"
                alignItems="center"
                spacing={0.25}
                sx={{ px: 1, pb: 1, minHeight: 40 }}
              >
                <Tooltip title="Slash command / skill">
                  <span>
                    {desktopShortcut(
                      <IconButton
                        size="small"
                        aria-label="slash command"
                        disabled={dead}
                        onClick={(): void =>
                          editorRef.current?.insertTrigger("/")}
                      >
                        <Box
                          component="span"
                          sx={{
                            fontSize: "1.1rem",
                            fontWeight: 700,
                            lineHeight: 1,
                          }}
                        >
                          /
                        </Box>
                      </IconButton>,
                      `${ALT_LABEL}/`,
                      `${ALT_LABEL}/ · slash command`,
                      !dead,
                    )}
                  </span>
                </Tooltip>
                <Tooltip title="Reference a file (@)">
                  <span>
                    {desktopShortcut(
                      <IconButton
                        size="small"
                        aria-label="reference a file"
                        disabled={dead}
                        onClick={(): void =>
                          editorRef.current?.insertTrigger("@")}
                      >
                        <AlternateEmail fontSize="small" />
                      </IconButton>,
                      `${ALT_LABEL}R`,
                      `${ALT_LABEL}R · reference a file`,
                      !dead,
                    )}
                  </span>
                </Tooltip>
                <Tooltip title="Attach image or file">
                  <span>
                    {desktopShortcut(
                      <IconButton
                        size="small"
                        aria-label="attach image or file"
                        disabled={dead}
                        onClick={(): void => fileInputRef.current?.click()}
                      >
                        <AttachFile fontSize="small" />
                      </IconButton>,
                      `${ALT_LABEL}A`,
                      `${ALT_LABEL}A · attach file`,
                      !dead,
                    )}
                  </span>
                </Tooltip>
                <Box sx={{ flex: 1 }} />

                {desktopActionsExpanded && (
                  <>
                    <Tooltip title="Save as draft">
                      <span>
                        {desktopShortcut(
                          <IconButton
                            size="small"
                            aria-label="save as draft"
                            disabled={!sendable}
                            onClick={saveDraft}
                          >
                            <EditNoteOutlined fontSize="small" />
                          </IconButton>,
                          `${MOD_LABEL}S`,
                          `${MOD_LABEL}S · save as draft`,
                          sendable,
                        )}
                      </span>
                    </Tooltip>
                    <Tooltip title="Schedule send">
                      <span>
                        {desktopShortcut(
                          <IconButton
                            size="small"
                            aria-label="schedule send"
                            disabled={!sendable}
                            onClick={(): void =>
                              setScheduleTarget({
                                id: undefined,
                                initial: null,
                              })}
                          >
                            <Schedule fontSize="small" />
                          </IconButton>,
                          `${ALT_LABEL}S`,
                          `${ALT_LABEL}S · schedule prompt`,
                          sendable,
                        )}
                      </span>
                    </Tooltip>
                    <Tooltip title="Jump to front of queue">
                      <span>
                        {desktopShortcut(
                          <IconButton
                            size="small"
                            aria-label="jump to front of queue"
                            disabled={!sendable || queue.length === 0}
                            onClick={jumpToFront}
                          >
                            <VerticalAlignTop fontSize="small" />
                          </IconButton>,
                          `${MOD_LABEL}J`,
                          `${MOD_LABEL}J · jump to front`,
                          sendable && queue.length > 0,
                        )}
                      </span>
                    </Tooltip>
                    <Tooltip title="Force push">
                      <span>
                        {desktopShortcut(
                          <IconButton
                            size="small"
                            color="warning"
                            aria-label="force push"
                            disabled={!sendable ||
                              !(busy || starting || paused)}
                            onClick={(e): void =>
                              setForceAnchor(e.currentTarget)}
                          >
                            <Bolt fontSize="small" />
                          </IconButton>,
                          `${ALT_LABEL}↵`,
                          `${ALT_LABEL}Enter · force push`,
                          sendable && (busy || starting || paused),
                        )}
                      </span>
                    </Tooltip>
                  </>
                )}

                {!desktopActionsExpanded && (
                  <Tooltip title="More delivery options">
                    <span>
                      {desktopShortcut(
                        <IconButton
                          ref={desktopMoreButtonRef}
                          size="small"
                          aria-label="more delivery options"
                          aria-controls={desktopMoreAnchor
                            ? "desktop-composer-more"
                            : undefined}
                          aria-expanded={desktopMoreAnchor ? "true" : undefined}
                          onClick={(e): void =>
                            setDesktopMoreAnchor(e.currentTarget)}
                        >
                          <MoreVert fontSize="small" />
                        </IconButton>,
                        `${MOD_LABEL}.`,
                        `${MOD_LABEL}. · more prompt actions`,
                      )}
                    </span>
                  </Tooltip>
                )}
                <Menu
                  id="desktop-composer-more"
                  anchorEl={desktopMoreAnchor}
                  open={desktopMoreAnchor !== null}
                  onClose={(): void => setDesktopMoreAnchor(null)}
                  anchorOrigin={{ vertical: "top", horizontal: "right" }}
                  transformOrigin={{ vertical: "bottom", horizontal: "right" }}
                >
                  {!desktopActionsExpanded && (
                    <MenuItem
                      disabled={!sendable}
                      onClick={(): void => {
                        setDesktopMoreAnchor(null);
                        saveDraft();
                      }}
                    >
                      <EditNoteOutlined fontSize="small" sx={{ mr: 1.25 }} />
                      Save as draft
                    </MenuItem>
                  )}
                  {!desktopActionsExpanded && (
                    <MenuItem
                      disabled={!sendable}
                      onClick={(): void => {
                        setDesktopMoreAnchor(null);
                        setScheduleTarget({ id: undefined, initial: null });
                      }}
                    >
                      <Schedule fontSize="small" sx={{ mr: 1.25 }} />
                      Schedule send
                    </MenuItem>
                  )}
                  {!desktopActionsExpanded && <Divider />}
                  {!desktopActionsExpanded && (
                    <MenuItem
                      disabled={!sendable || queue.length === 0}
                      onClick={(): void => {
                        setDesktopMoreAnchor(null);
                        jumpToFront();
                      }}
                    >
                      <VerticalAlignTop fontSize="small" sx={{ mr: 1.25 }} />
                      Jump to front of queue
                    </MenuItem>
                  )}
                  {!desktopActionsExpanded && (
                    <MenuItem
                      disabled={!sendable || !(busy || starting || paused)}
                      onClick={(): void => {
                        setDesktopMoreAnchor(null);
                        setForceAnchor(desktopMoreButtonRef.current);
                      }}
                    >
                      <Bolt
                        fontSize="small"
                        color="warning"
                        sx={{ mr: 1.25 }}
                      />
                      Force push…
                    </MenuItem>
                  )}
                </Menu>

                {desktopShortcut(
                  <Button
                    ref={queueBtnRef}
                    variant="contained"
                    size="small"
                    disableElevation
                    startIcon={<Send fontSize="small" />}
                    aria-label={busy || starting ? "queue message" : "send"}
                    disabled={!sendable || submitFeedback.pending}
                    aria-busy={submitFeedback.pending || undefined}
                    onClick={(): void => submitWithFeedback()}
                    sx={{
                      ml: 0.5,
                      minWidth: 86,
                      borderRadius: 1.5,
                      textTransform: "none",
                      fontWeight: 650,
                    }}
                  >
                    {submitFeedback.progress
                      ? <CircularProgress size={16} color="inherit" />
                      : busy || starting
                      ? "Queue"
                      : "Send"}
                  </Button>,
                  `${MOD_LABEL}↵`,
                  `${busy || starting ? "Queue" : "Send"} · ${MOD_LABEL}Enter`,
                  sendable,
                )}
              </Stack>
            </>
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
                  {!preparing && !desktop && compactAction && (
                    <Tooltip
                      title={compacting
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
                          disabled={dead || compacting}
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
                  {!preparing && !compact && (
                    <Tooltip title="Reference a file (@)">
                      <span>
                        <IconButton
                          aria-label="reference a file"
                          disabled={dead}
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
                  {!preparing && clearAction && (
                    <Tooltip title="Clear conversation">
                      <span>
                        <IconButton
                          aria-label="clear conversation"
                          disabled={dead}
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
                    /* Primary action: Send (idle) / Queue (busy — long-press → force push).
            Moved here from the old absolute overlay so the whole composer is one
            card; the long-press force-push ring + haptics are preserved. */
                  }
                  {preparing
                    ? (
                      <Tooltip title="Preparing session">
                        <span>
                          <IconButton
                            color="primary"
                            aria-label="preparing session"
                            disabled
                            sx={TOOLBAR_ICON_BTN}
                          >
                            <CircularProgress size={17} color="inherit" />
                          </IconButton>
                        </span>
                      </Tooltip>
                    )
                    : busy || starting
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
                          aria-label="queue message"
                          disabled={!sendable || submitFeedback.pending}
                          aria-busy={submitFeedback.pending || undefined}
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
                          {submitFeedback.progress
                            ? <CircularProgress size={18} color="inherit" />
                            : <Send fontSize="small" />}
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
                      <Tooltip title={`Send (${MOD_LABEL}${ENTER_LABEL})`}>
                        <span>
                          <IconButton
                            color="primary"
                            aria-label="send"
                            disabled={!sendable || submitFeedback.pending}
                            aria-busy={submitFeedback.pending || undefined}
                            sx={TOOLBAR_ICON_BTN}
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
                              ? <CircularProgress size={18} color="inherit" />
                              : <Send fontSize="small" />}
                          </IconButton>
                        </span>
                      </Tooltip>
                    )}
                  {
                    /* Secondary actions, always inline now: Save-draft (always), Jump-to-front
            (with a queue), Force-push (while busy/starting). The narrow-phone ⋮ fold
            is gone — moving the session-level controls (config / auto-scroll / Stop)
            out to the navbar freed the room that fold used to reclaim. */
                  }
                  {!preparing && (
                    <>
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
                      <Tooltip title="Force push">
                        <span>
                          <IconButton
                            color="warning"
                            aria-label="force push"
                            // Usable while a turn runs (busy/starting) OR the queue is paused —
                            // in both cases ⚡ pushes this message ahead of the held/queued work.
                            disabled={!sendable ||
                              !(busy || starting || paused)}
                            sx={TOOLBAR_ICON_BTN}
                            onPointerDown={(event): void =>
                              event.preventDefault()}
                            onClick={(e): void =>
                              setForceAnchor(e.currentTarget)}
                          >
                            <Bolt fontSize="small" />
                          </IconButton>
                        </span>
                      </Tooltip>
                      {
                        /* Jump-front + Force-push are ALWAYS shown so the send cluster never reflows
            as the queue fills/drains or a turn starts/ends; each is just disabled
            (greyed) when it doesn't apply — Jump-front with no queue to jump, Force-push
            with no running turn to interrupt. */
                      }
                      <Tooltip title="Jump to front of queue">
                        <span>
                          <IconButton
                            aria-label="jump to front of queue"
                            disabled={!sendable || queue.length === 0}
                            sx={TOOLBAR_ICON_BTN}
                            onClick={jumpToFront}
                          >
                            <VerticalAlignTop fontSize="small" />
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
                    </>
                  )}
                  <Tooltip title="Clear composer">
                    <span data-mobile-composer-clear>
                      <IconButton
                        aria-label="clear composer"
                        disabled={!clearable}
                        sx={TOOLBAR_ICON_BTN}
                        onPointerDown={(event): void => event.preventDefault()}
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
        /* Force-push confirm — opened by a completed long-press on Queue. Anchored
          to the button, rising above it. Confirm interrupts the running turn and
          runs this prompt next (skipping the queue). */
      }
      <Popper
        open={forceAnchor !== null}
        anchorEl={forceAnchor}
        placement="top"
        modifiers={[
          { name: "offset", options: { offset: [0, 8] } },
          {
            name: "flip",
            options: { fallbackPlacements: ["bottom", "right", "left"] },
          },
          { name: "preventOverflow", options: { padding: 8 } },
        ]}
        sx={{ zIndex: (theme) => theme.zIndex.modal }}
      >
        <ClickAwayListener onClickAway={(): void => setForceAnchor(null)}>
          <Paper
            role="dialog"
            aria-modal="false"
            aria-label="Force push confirmation"
            sx={{ maxWidth: 268, borderRadius: "12px", boxShadow: 8 }}
          >
            <Box sx={{ p: 1.5 }}>
              <Stack
                direction="row"
                spacing={0.75}
                alignItems="center"
                sx={{ mb: 0.5 }}
              >
                <Bolt fontSize="small" color="warning" />
                <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
                  Force push
                </Typography>
              </Stack>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ display: "block", lineHeight: 1.5 }}
              >
                {busy || starting
                  ? "Interrupt the current turn and run this now, skipping the queue."
                  : "Send this now, ahead of the paused queue."}
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
                  onPointerDown={(event): void => event.preventDefault()}
                  onClick={(): void => setForceAnchor(null)}
                >
                  Cancel
                  <Kbd keys="Esc" />
                </Button>
                <NetworkButton
                  size="small"
                  variant="contained"
                  color="warning"
                  startIcon={<Bolt />}
                  onPointerDown={(event): void => event.preventDefault()}
                  networkAction={confirmForce}
                >
                  Force push
                  <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
                </NetworkButton>
              </Stack>
            </Box>
          </Paper>
        </ClickAwayListener>
      </Popper>
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
                onPointerDown={(event): void => {
                  event.preventDefault();
                  imagePreviewTap.onPointerDown(event);
                }}
                onPointerMove={imagePreviewTap.onPointerMove}
                onPointerUp={imagePreviewTap.onPointerUp}
                onPointerCancel={imagePreviewTap.onPointerCancel}
                onClick={imagePreviewTap.onClick}
                sx={INLINE_IMAGE_ACTION_BUTTON_SX}
              >
                Preview
              </Button>
              <Button
                color="error"
                startIcon={<DeleteOutline />}
                onPointerDown={(event): void => {
                  event.preventDefault();
                  imageDeleteTap.onPointerDown(event);
                }}
                onPointerMove={imageDeleteTap.onPointerMove}
                onPointerUp={imageDeleteTap.onPointerUp}
                onPointerCancel={imageDeleteTap.onPointerCancel}
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
          onSubmit={(): void => submitWithFeedback(() => setComposeFs(false))}
          onSaveDraft={(): void => {
            saveDraft();
            setComposeFs(false);
          }}
          onSchedule={(): void =>
            setScheduleTarget({ id: undefined, initial: null })}
          onForcePush={(anchor): void => setForceAnchor(anchor)}
          forcePushEnabled={busy || starting || paused}
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
          placeholder={dead
            ? "Send to resume this session…"
            : "Message the agent…"}
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
          send({ type: "cancel", session_id: sessionId });
          setCancelOpen(false);
        }}
      />
      {
        /* Move-draft destination picker + undo snackbar. Owned here (not in the
          drafts panel) so the snackbar survives when moving the LAST draft
          unmounts that panel. */
      }
      <Sheet
        open={moveSrcId !== null}
        onClose={(): void => setMoveSrcId(null)}
        title="Move draft to…"
        mobileDismiss="footer"
        portal
      >
        {
          /* Bleed the list to the card so rows share the title inset. A
            <button> with nowrap cwd otherwise grows to min-content and
            paints through the right pad; width 100% + flex 0 basis keeps
            both sides on the same 18px gutter. */
        }
        <List disablePadding sx={{ mx: -2.25, pb: 1 }}>
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
              sx={{
                px: 2.25,
                py: 1.25,
                width: "100%",
                maxWidth: "100%",
                minWidth: 0,
                overflow: "hidden",
                boxSizing: "border-box",
              }}
            >
              <ListItemText
                primary={s.title}
                secondary={s.cwd}
                sx={{
                  my: 0,
                  minWidth: 0,
                  flex: "1 1 0px",
                  overflow: "hidden",
                  "& .MuiListItemText-primary, & .MuiListItemText-secondary": {
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  },
                }}
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
        // MUI's default Mobile top=8 sits entirely behind an iPhone status bar
        // (only its bottom shadow remains visible). Keep the Undo toast inside
        // both safe-area and horizontal viewport bounds.
        sx={{
          top: {
            xs: "calc(env(safe-area-inset-top, 0px) + 8px)",
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
const QueuedAttachmentChips = memo(function QueuedAttachmentChips({
  attachments,
}: {
  attachments: Attachment[];
}): React.JSX.Element {
  return (
    <Box
      sx={{ display: "inline-flex", flexWrap: "wrap", gap: 0.5, mt: 0.5 }}
    >
      {attachments.map((a, i) =>
        a.isImage && a.previewUrl
          ? (
            <Box
              key={a.id}
              component="img"
              src={a.previewUrl}
              alt={a.name}
              data-pending-content-action="attachment-preview"
              onClick={(event): void => {
                event.stopPropagation();
                openLightbox(attachments, i);
              }}
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
              data-pending-content-action="attachment-preview"
              onClick={(event): void => {
                event.stopPropagation();
                openLightbox(attachments, i);
              }}
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
}, (a, b) =>
  a.attachments.length === b.attachments.length &&
  a.attachments.every((x, i) => x.id === b.attachments[i]?.id));

// The Zed-style staging panel above the editor — one component for two kinds:
//   - "queued": prompts the busy agent can't take yet, auto-drained one per turn.
//   - "draft":  parked messages the user holds; activated (sent/queued) on demand.

const uploadPulse = keyframes`
  0%, 100% { opacity: 0.42; transform: translateY(0); }
  50% { opacity: 1; transform: translateY(-1px); }
`;

// An OPTIMISTIC draft/queue row: shown the instant you stage or move it, before
// the service confirms. Connected `pending` stays quiet so a fast LAN confirm
// never flashes a loader. Disconnected `pending` is an upload waiting to reach
// the service. `sending` is in-flight. `failed` (retry still offline, or ack
// timeout) is red with retry / return / discard.
function OptimisticDraftRow({
  sessionId,
  kind,
  message,
}: {
  sessionId: string;
  kind: "queued" | "draft";
  message: QueuedMessage;
}): React.JSX.Element {
  const connected = useConnected();
  const appearance = pendingSyncAppearance(message.status, connected);
  const failed = appearance === "failed";
  const sending = appearance === "sending";
  const syncing = appearance === "syncing";
  const cmid = message.cmid ?? "";
  const previewText = message.text;
  const previewTray = attachmentTrayForSurface(message.attachments, previewText);
  const canReturn = failed && canReturnFromPendingRow(kind, message.origin);
  const returnHome = homeForOrigin(message.origin ?? (kind === "queued" ? "draft" : "composer"));
  return (
    <Paper
      elevation={0}
      sx={(t) => ({
        p: 0.75,
        display: "flex",
        alignItems: "flex-start",
        minHeight: mobilePendingRowMinHeight,
        gap: 0.5,
        bgcolor: failed
          ? alpha(t.palette.error.main, 0.06)
          : sending
          ? alpha(t.palette.primary.main, 0.05)
          : syncing
          ? alpha(t.palette.info.main, 0.06)
          : "background.paper",
        ...(failed && { borderLeft: `3px solid ${t.palette.error.main}` }),
      })}
    >
      {sending && (
        <CircularProgress
          size={13}
          thickness={5}
          sx={{ color: "primary.main", mt: 0.25, flexShrink: 0 }}
        />
      )}
      {syncing && (
        <CloudUpload
          aria-hidden
          sx={{
            fontSize: 16,
            color: "info.main",
            mt: 0.25,
            flexShrink: 0,
            animation: `${uploadPulse} 1.4s ease-in-out infinite`,
            "@media (prefers-reduced-motion: reduce)": { animation: "none" },
          }}
        />
      )}
      <Box sx={{ flex: 1, minWidth: 0 }}>
        {previewText.trim() !== ""
          ? (
            <MessagePreview
              text={previewText}
              attachments={message.attachments}
            />
          )
          : previewTray.length === 0
          ? (
            <Typography variant="body2" sx={{ color: "text.secondary" }}>
              📎 attachment
            </Typography>
          )
          : null}
        {syncing && (
          <Typography variant="caption" sx={{ color: "info.main" }}>
            Waiting to sync
          </Typography>
        )}
        {sending && (
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            Sending…
          </Typography>
        )}
        {failed && (
          <Typography variant="caption" sx={{ color: "error.main" }}>
            Couldn't reach Cowboy
          </Typography>
        )}
        {previewTray.length > 0 && (
          <QueuedAttachmentChips attachments={previewTray} />
        )}
      </Box>
      {kind === "draft" && !failed && !sending && (
        <Tooltip title="Send">
          <IconButton
            size="small"
            color="primary"
            aria-label="send draft"
            onClick={(): void => {
              haptic();
              void activateDraft(sessionId, message.id);
            }}
          >
            <Send fontSize="small" />
          </IconButton>
        </Tooltip>
      )}
      {failed && (
        <Stack direction="row" sx={{ flexShrink: 0 }}>
          <Tooltip title="Retry">
            <IconButton
              size="small"
              aria-label="retry send"
              onClick={(): void => {
                haptic();
                retryQueued(sessionId, cmid);
              }}
            >
              <Refresh fontSize="small" />
            </IconButton>
          </Tooltip>
          {canReturn && (
            <Tooltip title={returnLabelForHome(returnHome)}>
              <IconButton
                size="small"
                aria-label={returnLabelForHome(returnHome)}
                onClick={(): void => {
                  haptic();
                  returnFailedQueued(sessionId, cmid);
                }}
              >
                <Undo fontSize="small" />
              </IconButton>
            </Tooltip>
          )}
          <Tooltip title="Discard">
            <IconButton
              size="small"
              aria-label="discard"
              onClick={(): void => {
                importantHaptic();
                discardQueued(sessionId, cmid);
              }}
            >
              <Close fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>
      )}
    </Paper>
  );
}

interface PendingEditController {
  isDirty: () => boolean;
  save: () => void;
  discard: () => void;
}

// Collapsible header ("N Queued Messages" / "N Drafts"). Bulk Send all /
// Clear all live in the header kebab so the collapsed bar stays one line.
// Drafts sit BELOW the queue and above the composer (see the Composer render).
function PendingPanel({
  desktop,
  keyboardOpen,
  kind,
  sessionId,
  items,
  status,
  commands,
  onMoveDraft,
  onScheduleDraft,
  onEditingChange,
  onResumeKeyboardSurface,
}: {
  desktop: boolean;
  keyboardOpen: boolean;
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
  /** Open the schedule picker for a draft (draft kind only) — set/reschedule/
      cancel its future auto-send. Owned by the Composer (survives unmount). */
  onScheduleDraft?: ((id: string) => void) | undefined;
  /** Mobile parent projection used to yield the ordinary composer while this
   *  panel owns the only active writing surface. */
  onEditingChange?: ((editing: boolean) => void) | undefined;
  /** Clear the compact-surface latch so a Queue/Draft tap can own the
   *  keyboard-up two-track chrome after delivery or hide-keyboard. */
  onResumeKeyboardSurface?: (() => void) | undefined;
}): React.JSX.Element {
  const scrollRef = useRef<HTMLDivElement>(null);
  // Plan / Queue / Draft share one exclusive accordion: opening this panel
  // folds the other two; tapping the already-open panel folds all three.
  const collapsed = usePrefStore(composerStackExpandedStore()) !== kind;
  const [arrivalFlash, setArrivalFlash] = useState<PendingArrival | null>(null);
  // One viewport/focus observer per panel, not per message row. A long queue can
  // contain dozens of rows; registering the iOS keyboard listeners in every row
  // makes one dismissal fan out through unnecessary handlers and causes jank.
  const [editingId, setEditingId] = useState<string | null>(null);
  const editControllerRef = useRef<PendingEditController | null>(null);
  const suppressEditFocusRestoreRef = useRef(false);
  const registerEditController = useCallback(
    (controller: PendingEditController | null): void => {
      editControllerRef.current = controller;
    },
    [],
  );
  const [confirmCollapseEdit, setConfirmCollapseEdit] = useState(false);
  const [panelMenuEl, setPanelMenuEl] = useState<HTMLElement | null>(null);
  const [bulkConfirm, setBulkConfirm] = useState<"send-all" | "clear-all" | null>(
    null,
  );
  // Accordion disclosure is a view preference; edit ownership is a transaction.
  // A transaction always wins visually so persisted/local disclosure state can
  // never hide the only active Mobile composer.
  const visuallyCollapsed = collapsed && editingId === null;
  const settleEditAndCollapse = (resolution: "save" | "discard"): void => {
    const controller = editControllerRef.current;
    if (controller === null) return;
    suppressEditFocusRestoreRef.current = true;
    if (resolution === "save") controller.save();
    else controller.discard();
    setConfirmCollapseEdit(false);
    collapseComposerStackPanel(kind);
  };
  useConfirmEnter(confirmCollapseEdit, () => settleEditAndCollapse("save"));
  const runBulkConfirm = async (): Promise<void> => {
    if (bulkConfirm === "send-all") {
      navigationHaptic();
      await activateAllDrafts(sessionId);
    } else if (bulkConfirm === "clear-all") {
      importantHaptic();
      if (kind === "queued") await clearQueue(sessionId);
      else await clearDrafts(sessionId);
    }
    setBulkConfirm(null);
  };
  useConfirmEnter(bulkConfirm !== null, () => {
    void runBulkConfirm();
  });
  useEffect(() => {
    return subscribePendingArrival((arrival) => {
      if (arrival.kind !== kind) return;
      setArrivalFlash(arrival);
    });
  }, [kind]);
  const revealArrivalRow = useCallback((): void => {
    if (arrivalFlash === null) return;
    const row = scrollRef.current?.querySelector<HTMLElement>(
      '[data-pending-row-flash="true"]',
    );
    if (row) scrollPendingRowIntoView(row);
  }, [arrivalFlash]);
  useEffect(() => {
    if (arrivalFlash === null) return undefined;
    const timer = globalThis.setTimeout(
      () => setArrivalFlash(null),
      PENDING_ARRIVAL_FLASH_MS,
    );
    let laterFrame = 0;
    let imageFrame = 0;
    const frame = globalThis.requestAnimationFrame(() => {
      laterFrame = globalThis.requestAnimationFrame(revealArrivalRow);
    });
    const settle = globalThis.setTimeout(revealArrivalRow, 180);
    const lateSettle = globalThis.setTimeout(revealArrivalRow, 420);
    const list = scrollRef.current;
    const onImageLoad = (event: Event): void => {
      const row = list?.querySelector<HTMLElement>(
        '[data-pending-row-flash="true"]',
      );
      if (
        row === null || row === undefined ||
        !(event.target instanceof HTMLImageElement) ||
        !row.contains(event.target)
      ) return;
      globalThis.cancelAnimationFrame(imageFrame);
      imageFrame = globalThis.requestAnimationFrame(revealArrivalRow);
    };
    list?.addEventListener("load", onImageLoad, true);
    return (): void => {
      globalThis.clearTimeout(timer);
      globalThis.clearTimeout(settle);
      globalThis.clearTimeout(lateSettle);
      globalThis.cancelAnimationFrame(frame);
      if (laterFrame !== 0) globalThis.cancelAnimationFrame(laterFrame);
      if (imageFrame !== 0) globalThis.cancelAnimationFrame(imageFrame);
      list?.removeEventListener("load", onImageLoad, true);
    };
  }, [arrivalFlash, revealArrivalRow]);
  const toggleCollapsed = (): void => {
    // Explicit haptic: the collapse/expand header is a custom clickable row, NOT a
    // MuiButtonBase, so the global delegation doesn't see it. Light disclosure tap.
    haptic();
    const decision = pendingPanelDisclosureDecision({
      collapsed,
      editing: editingId !== null,
      // Missing controller is treated as dirty: a transient registration race
      // must fail safe and keep the edit visible rather than lose its buffer.
      dirty: editControllerRef.current?.isDirty() ?? true,
    });
    if (decision === "confirm-dirty-edit") {
      setConfirmCollapseEdit(true);
      return;
    }
    if (decision === "discard-clean-edit-and-collapse") {
      settleEditAndCollapse("discard");
      return;
    }
    toggleComposerStackPanel(kind);
    if (desktop && decision === "expand") {
      requestAnimationFrame(() =>
        scrollRef.current?.querySelector<HTMLElement>("[data-desktop-item]")
          ?.focus({ preventScroll: true })
      );
    }
  };
  const toggleTap = useReliableTouchTap<HTMLButtonElement>(toggleCollapsed);
  useEffect(() => {
    onEditingChange?.(editingId !== null);
    return (): void => onEditingChange?.(false);
  }, [editingId, onEditingChange]);
  useEffect(() => {
    if (editingId !== null && !items.some((item) => item.id === editingId)) {
      editControllerRef.current = null;
      setConfirmCollapseEdit(false);
      setEditingId(null);
    }
  }, [editingId, items]);
  // Reorder is a low-frequency action, so the per-row drag grips are hidden by
  // default (they'd waste ~40px on every row of a narrow phone) and revealed
  // only in this opt-in "reorder mode" (iOS list-Edit pattern). Local + ephemeral
  // like `collapsed`. Per-panel state → drafts and queue toggle independently.
  const [reordering, setReordering] = useState(false);
  const count = items.length;
  // The manual queue pause holds the QUEUE drain only (drafts never auto-drain),
  // so the "Paused" badge shows on the queued panel — that's why its messages
  // aren't advancing. The pause/resume CONTROL itself is session-level and lives
  // in the navbar (AutoScrollAndStop), reachable even with an empty queue; here we
  // only surface the status badge. Read live so it tracks the toggle.
  const queueHeld = useStoreSelector((snapshot) =>
    kind === "queued" &&
    (snapshot.sessions.find((session) => session.id === sessionId)?.paused ??
      false)
  );
  // Reordering 0/1 items is meaningless — drop out of the mode (and hide its
  // toggle) so a cleared/sent-down panel never sits stuck in an empty mode.
  useEffect(() => {
    if (count < 2 && reordering) setReordering(false);
  }, [count, reordering]);
  useLayoutEffect(() => {
    const list = scrollRef.current;
    const region = list?.closest<HTMLElement>(
      `[data-desktop-region='prompt.${kind}']`,
    );
    if (!desktop || !list || !region) return undefined;
    region.dataset.desktopReordering = reordering ? "true" : "false";
    return undefined;
  }, [desktop, kind, reordering]);
  useEffect(() => {
    const list = scrollRef.current;
    const region = list?.closest<HTMLElement>(
      `[data-desktop-region='prompt.${kind}']`,
    );
    if (!desktop || !list || !region) return undefined;
    const onToggle = (): void => {
      const next = region.dataset.desktopReordering !== "true";
      region.dataset.desktopReordering = next ? "true" : "false";
      setReordering(next);
    };
    const onRelease = (): void => {
      region.dataset.desktopReordering = "false";
      setReordering(false);
    };
    list.addEventListener("cowboy:desktop-toggle-reorder", onToggle);
    list.addEventListener("cowboy:desktop-release-reorder", onRelease);
    return () => {
      delete region.dataset.desktopReordering;
      list.removeEventListener("cowboy:desktop-toggle-reorder", onToggle);
      list.removeEventListener("cowboy:desktop-release-reorder", onRelease);
    };
  }, [desktop, kind]);
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
    scrollContainer: () => scrollRef.current,
    onReorder: (order) =>
      kind === "queued"
        ? reorderQueue(sessionId, order)
        : reorderDrafts(sessionId, order),
    onDragStart: (): void => {
      // Pickup haptic is fired centrally in useSortable (covers every list). Here
      // we only need the QUEUE-specific hold: holding the head pauses the drain.
      if (kind === "queued") {
        const head = items[0];
        if (head) setQueueEditing(sessionId, head.id);
      }
    },
    onDragEnd: kind === "queued"
      ? (): void => setQueueEditing(sessionId, null)
      : undefined,
  });
  useEffect(() => {
    const list = scrollRef.current;
    if (!desktop || !list) return undefined;
    const onKeyboardReorder = (event: Event): void => {
      const detail = (event as CustomEvent<{ delta?: number }>).detail;
      const item = event.target instanceof Element
        ? event.target.closest<HTMLElement>("[data-desktop-item]")
        : null;
      const id = item?.dataset.desktopItem;
      const delta = detail?.delta;
      if (!id || (delta !== -1 && delta !== 1)) return;
      const current = sortable.order.indexOf(id);
      const next = Math.max(
        0,
        Math.min(sortable.order.length - 1, current + delta),
      );
      if (current < 0 || current === next) return;
      const order = [...sortable.order];
      order.splice(current, 1);
      order.splice(next, 0, id);
      if (kind === "queued") reorderQueue(sessionId, order);
      else reorderDrafts(sessionId, order);
      requestAnimationFrame(() =>
        list.querySelector<HTMLElement>(
          `[data-desktop-item="${CSS.escape(id)}"]`,
        )?.focus({ preventScroll: true })
      );
    };
    list.addEventListener("cowboy:desktop-reorder", onKeyboardReorder);
    return () =>
      list.removeEventListener("cowboy:desktop-reorder", onKeyboardReorder);
  }, [desktop, kind, sessionId, sortable.order]);
  const noun = kind === "queued" ? "Queued Message" : "Draft";
  const mobileFloatingEdit = !desktop && editingId !== null && keyboardOpen;
  return (
    <Box
      data-mobile-pending-panel={!desktop ? kind : undefined}
      data-mobile-floating-edit={mobileFloatingEdit ? "true" : undefined}
      {...(desktop
        ? {
          "data-desktop-region": `prompt.${kind}`,
          "data-desktop-reorderable": "true",
          "data-desktop-focus-default": true,
          tabIndex: -1,
        }
        : {})}
      sx={{
        mb: desktop ? 1 : 0,
        // The original framed container, KEPT (the user liked it): a soft tinted,
        // rounded, bordered box that groups the rows. Its OUTER edge sits at the
        // composer's content gutter — exactly where the input box's outer border is
        // — so the panel frame and the message box line up edge-to-edge (no horizontal
        // margin on either). Drafts read a touch more "staging" than the live queue.
        ...(desktop
          ? desktopSurfaceSx({ interactive: false, focusWithin: true })
          : mobileComposerPanelFrameSx),
        bgcolor: kind === "draft" ? "action.selected" : "action.hover",
        ...(mobileFloatingEdit && {
          border: 0,
          bgcolor: "transparent",
          boxShadow: "none",
          "& [data-mobile-pending-row]:not([data-mobile-pending-row-editing='true'])":
            {
              display: "none",
            },
        }),
        ...(desktop && {
          flexShrink: 0,
        }),
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
        data-mobile-pending-header={!desktop ? "true" : undefined}
        direction="row"
        alignItems="center"
        // Pin the header to the SAME 44px as the composer input (ComposerTextarea
        // `MuiInputBase-root` minHeight) so the "N Drafts" bar and the message box
        // read as the same-height pair. `py: 0` drops the old extra 8px that made
        // the bar (a 44px icon button + padding) taller than the input.
        sx={{
          display: mobileFloatingEdit ? "none" : "flex",
          pr: 0.75,
          py: 0,
          minHeight: mobileComposerPanelHeaderMinHeight,
        }}
      >
        {
          /* Match PlanDock: the whole summary area is one disclosure button, not
            just the chevron/label. Bulk and reorder actions stay separate sibling
            controls, so tapping them never also folds the panel. */
        }
        <ButtonBase
          {...toggleTap}
          disableRipple
          data-desktop-item-action="default"
          data-desktop-collapse-toggle={desktop ? kind : undefined}
          aria-label={editingId !== null
            ? `Finish editing ${noun.toLowerCase()}`
            : `${
              visuallyCollapsed ? "Expand" : "Collapse"
            } ${noun.toLowerCase()}s`}
          aria-expanded={!visuallyCollapsed}
          sx={{
            alignSelf: "stretch",
            flex: 1,
            minWidth: 0,
            justifyContent: "flex-start",
            textAlign: "left",
            pl: 0.5,
            touchAction: "manipulation",
            // MUI's touch ripple can remain painted on iOS when this reliable
            // pointerup activation changes the panel layout before Safari emits
            // its follow-up click. Keep immediate press feedback without a
            // stateful ripple that can get stranded after the panel expands.
            "&:active": { bgcolor: "action.hover" },
          }}
        >
          <Box
            sx={{
              width: 40,
              display: "inline-flex",
              justifyContent: "center",
              flexShrink: 0,
            }}
          >
            {editingId !== null
              ? <EditOutlined fontSize="small" color="primary" />
              : visuallyCollapsed
              ? <ChevronRight fontSize="small" />
              : <ExpandMore fontSize="small" />}
          </Box>
          <Typography variant="caption" sx={{ fontWeight: 600, minWidth: 0 }}>
            {count} {noun}
            {count === 1 ? "" : "s"}
          </Typography>
          {editingId !== null && (
            <Typography
              variant="caption"
              color="primary.main"
              sx={{ ml: 0.75, fontWeight: 700, flexShrink: 0 }}
            >
              Editing
            </Typography>
          )}
          {desktop && (
            <Suspense
              fallback={
                <ShortcutKeycap
                  keyLabel="G"
                  variant="context"
                  availability="inactive"
                  sx={{ ml: 0.75 }}
                />
              }
            >
              <DesktopListJumpKeycap
                region={`prompt.${kind}`}
                keyLabel="G"
                prefix
                sx={{ ml: 0.75 }}
              />
            </Suspense>
          )}
          {
            /* Why-it's-held badge: the queue is manually paused, so it won't drain
              until the user resumes (the ⏸ toggle in the nav/status bar). */
          }
          {queueHeld && (
            <Box
              sx={{
                display: "inline-flex",
                alignItems: "center",
                gap: 0.25,
                ml: 0.75,
                px: 0.625,
                py: 0.125,
                borderRadius: 1,
                bgcolor: "warning.main",
                color: "warning.contrastText",
                flexShrink: 0,
              }}
            >
              <Typography
                variant="caption"
                sx={{ fontWeight: 700, lineHeight: 1.2 }}
              >
                Paused
              </Typography>
            </Box>
          )}
          <Box sx={{ flex: 1, minWidth: 0 }} />
          {desktop && (
            <Stack direction="row" spacing={0.5} alignItems="center">
              {kind === "queued" && (
                <Suspense fallback={null}>
                  <DesktopRegionShortcut
                    shortcut="Mod+Y"
                    title={visuallyCollapsed
                      ? "Open and focus queue"
                      : "Close queue when focused"}
                    singleKeycap={`${MOD_LABEL}Y`}
                  />
                </Suspense>
              )}
              {kind === "draft" && (
                <Suspense fallback={null}>
                  <DesktopRegionShortcut
                    shortcut="Mod+D"
                    title="Focus drafts"
                    singleKeycap={`${MOD_LABEL}D`}
                  />
                </Suspense>
              )}
            </Stack>
          )}
          {desktop && reordering && (
            <Typography
              variant="caption"
              color="primary.main"
              sx={{ ml: 0.75, fontWeight: 700, letterSpacing: "0.04em" }}
            >
              REORDER · J/K MOVE · ESC DONE
            </Typography>
          )}
        </ButtonBase>
        {
          /* Reorder toggle — reveals the per-row drag grips. Only meaningful (and
            only shown) with 2+ rows. Primary-tinted while active. HIDDEN on a wide
            panel (ROW_ACTIONS_INLINE): there the grips are always shown, so the
            toggle is redundant — same adaptive rule as the row actions. */
        }
        {count >= 2 && (
          <IconButton
            size="small"
            disabled={editingId !== null}
            aria-label={reordering ? "done reordering" : "reorder"}
            title={reordering ? "Done" : "Reorder"}
            color={reordering ? "primary" : "default"}
            onClick={(): void =>
              setReordering((r) => {
                if (!r) haptic(); // light — entering reorder mode (grips now live)
                return !r;
              })}
            sx={{ flexShrink: 0, [ROW_ACTIONS_INLINE]: { display: "none" } }}
          >
            <SwapVert fontSize="small" />
          </IconButton>
        )}
        <IconButton
          size="small"
          disabled={editingId !== null}
          aria-label={kind === "draft" ? "Draft actions" : "Queue actions"}
          onClick={(event): void => {
            haptic();
            setPanelMenuEl(event.currentTarget);
          }}
          sx={{ flexShrink: 0 }}
        >
          <MoreVert fontSize="small" />
        </IconButton>
        <Menu
          open={panelMenuEl !== null}
          anchorEl={panelMenuEl}
          onClose={(): void => setPanelMenuEl(null)}
          anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
          transformOrigin={{ vertical: "top", horizontal: "right" }}
          disableScrollLock
        >
          {kind === "draft" && (
            <MenuItem
              disabled={editingId !== null}
              onClick={(): void => {
                setPanelMenuEl(null);
                setBulkConfirm("send-all");
              }}
            >
              <ListItemIcon><Send fontSize="small" /></ListItemIcon>
              <ListItemText>Send all</ListItemText>
            </MenuItem>
          )}
          <MenuItem
            disabled={editingId !== null}
            onClick={(): void => {
              setPanelMenuEl(null);
              setBulkConfirm("clear-all");
            }}
          >
            <ListItemIcon><DeleteOutline fontSize="small" /></ListItemIcon>
            <ListItemText>Clear all</ListItemText>
          </MenuItem>
        </Menu>
        <ConfirmSheet
          open={bulkConfirm !== null}
          onClose={(): void => setBulkConfirm(null)}
          title={bulkConfirm === "send-all" ? "Send all drafts?" : kind === "queued"
            ? "Clear all queued messages?"
            : "Clear all drafts?"}
          actions={
            <>
              <Button
                color="inherit"
                onClick={(): void => setBulkConfirm(null)}
                sx={{ textTransform: "none" }}
              >
                Cancel
                <Kbd keys="Esc" />
              </Button>
              <NetworkButton
                variant="contained"
                color={bulkConfirm === "send-all" ? "primary" : "error"}
                networkAction={runBulkConfirm}
                sx={{ textTransform: "none" }}
              >
                {bulkConfirm === "send-all" ? "Send all" : "Clear all"}
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </NetworkButton>
            </>
          }
        >
          <DialogContentText>
            {bulkConfirm === "send-all"
              ? `Send all ${String(count)} drafts to the agent?`
              : kind === "queued"
              ? `Clear all ${String(count)} queued messages?`
              : `Clear all ${String(count)} drafts?`}
          </DialogContentText>
        </ConfirmSheet>
      </Stack>
      {
        /* Match PlanDock's disclosure motion. Non-editing rows stay mounted for
          preview/sort stability, but an active edit forces the panel open until
          its transaction is saved or discarded. Hiding an unresolved edit would
          strand Mobile's sole composer ownership and remove the main input too. */
      }
      <Collapse
        in={!visuallyCollapsed}
        onEntered={revealArrivalRow}
        sx={{
          willChange: "height",
          // Large attachment previews otherwise re-enter layout + paint on
          // every clipped-height frame. Isolate the already-laid-out list so
          // the compositor can reveal it as smoothly as Plan's text-only list.
          // Layout containment can leave iOS WebKit's parent overflow layer
          // with the pre-animation scroll range until a later layout pass.
          // That presents as a temporarily frozen Queue/Drafts stack after a
          // disclosure. Desktop benefits from the paint isolation; Mobile's
          // native scrollport must observe every intermediate/final height.
          ...(desktop && {
            "& .MuiCollapse-wrapperInner": { contain: "layout paint" },
          }),
        }}
      >
        <Stack
          spacing={0.5}
          ref={scrollRef}
          data-mobile-pending-scrollport={!desktop && !visuallyCollapsed
            ? "true"
            : undefined}
          data-desktop-pending-list={desktop ? "true" : undefined}
          data-desktop-aux-list={desktop ? "true" : undefined}
          sx={{
            // Inner padding so the rows sit INSIDE the frame with a small inset
            // (the original framed look). The frame's OUTER edge is what aligns
            // with the input box, not the rows.
            px: mobileFloatingEdit ? 0 : 0.5,
            pb: mobileFloatingEdit ? 0 : 0.5,
            // Exclusive accordion means only one body is open, so each panel
            // owns its own cap + scrollbar. Keyboard-up floating edit is the
            // writing surface and must not clip the editor at 30vh.
            // Collapse keeps children mounted. A hidden
            // `-webkit-overflow-scrolling: touch` tile still becomes its own
            // iOS compositor scroller and freezes the transcript until reload.
            ...(!desktop && !mobileFloatingEdit && {
              maxHeight: "30vh",
              overflowY: visuallyCollapsed ? "hidden" : "auto",
              overscrollBehavior: visuallyCollapsed ? "auto" : "contain",
              WebkitOverflowScrolling: visuallyCollapsed ? "auto" : "touch",
            }),
            ...(desktop && {
              maxHeight: 176,
              overflowY: "auto",
              overscrollBehavior: "contain",
              transition: "max-height 150ms ease, padding 150ms ease",
              "[data-desktop-focused='true'] &": {
                maxHeight: "min(52vh, 640px)",
              },
            }),
          }}
        >
          {sortable.order.map((id, index) => {
            const m = byId.get(id);
            if (!m) return null;
            const jumpKey = desktop ? listJumpKey(index) : null;
            // A LOCAL optimistic draft (carries `status`) renders a lightweight
            // row with no grip / edit / reorder — it isn't a server item yet.
            const optimistic = m.status !== undefined;
            const leadingHandle = editingId !== m.id && !optimistic &&
              count > 1;
            const jumpBadgeSx = {
              position: "absolute",
              zIndex: 1,
              pointerEvents: desktop ? "auto" : "none",
              ...(reordering ? { top: 0, right: 0 } : {
                top: "50%",
                left: "50%",
                transform: "translate(-50%, -50%)",
              }),
              [ROW_ACTIONS_INLINE]: {
                top: 0,
                right: 0,
                left: "auto",
                transform: "none",
              },
            } as const;
            const arrivalFlashActive = pendingRowMatchesArrival(
              m,
              arrivalFlash,
            );
            return (
              <Stack
                key={m.id}
                data-mobile-pending-row={!desktop ? "true" : undefined}
                data-mobile-pending-row-editing={!desktop && editingId === m.id
                  ? "true"
                  : undefined}
                data-pending-row-flash={arrivalFlashActive
                  ? "true"
                  : undefined}
                {...(desktop
                  ? {
                    "data-desktop-item": m.id,
                    "data-desktop-current": editingId === m.id
                      ? "true"
                      : undefined,
                    tabIndex: -1,
                  }
                  : {})}
                ref={sortable.registerItem(m.id)}
                style={sortable.itemStyle(m.id)}
                direction="row"
                alignItems="center"
                spacing={0.5}
                sx={desktop
                  ? {
                    ...desktopListItemSx(),
                  }
                  : undefined}
              >
                {
                  /* One leading slot owns both reorder and the G+number jump hint.
                    On wide rows the number overlays the six-dot grip instead of
                    consuming a separate column. Narrow Desktop rows retain a quiet
                    28px jump hint until reorder mode reveals the 44px grip; Mobile
                    still allocates nothing until its reorder mode is active. A
                    single row needs neither ordering affordance nor ordinal. */
                }
                {leadingHandle && (
                  <Box
                    sx={{
                      position: "relative",
                      width: reordering ? 44 : (desktop ? 28 : 44),
                      height: 44,
                      flexShrink: 0,
                      display: reordering || desktop ? "inline-flex" : "none",
                      [ROW_ACTIONS_INLINE]: {
                        display: "inline-flex",
                        width: 44,
                      },
                    }}
                  >
                    <IconButton
                      {...sortable.handleProps(m.id)}
                      aria-label="Drag to reorder"
                      sx={{
                        ...TOOLBAR_ICON_BTN,
                        position: "absolute",
                        inset: 0,
                        width: 44,
                        height: 44,
                        color: "text.disabled",
                        display: reordering ? "inline-flex" : "none",
                        [ROW_ACTIONS_INLINE]: { display: "inline-flex" },
                      }}
                    >
                      <DragIndicator fontSize="small" />
                    </IconButton>
                    {jumpKey && (
                      <Suspense
                        fallback={
                          <ShortcutKeycap
                            keyLabel={jumpKey}
                            variant="context"
                            availability="inactive"
                            sx={jumpBadgeSx}
                          />
                        }
                      >
                        <DesktopListJumpKeycap
                          region={`prompt.${kind}`}
                          keyLabel={jumpKey}
                          sx={jumpBadgeSx}
                        />
                      </Suspense>
                    )}
                  </Box>
                )}
                <Box
                  sx={{
                    flex: 1,
                    minWidth: 0,
                    borderRadius: 1,
                    outline: "2px solid",
                    outlineColor: arrivalFlashActive
                      ? "primary.main"
                      : "transparent",
                    transition: (theme) =>
                      theme.transitions.create("outline-color", {
                        duration: 400,
                      }),
                    "@media (prefers-reduced-motion: reduce)": {
                      transition: "none",
                    },
                  }}
                >
                  {optimistic
                    ? <OptimisticDraftRow sessionId={sessionId} kind={kind} message={m} />
                    : (
                      <PendingRow
                        desktop={desktop}
                        kind={kind}
                        sessionId={sessionId}
                        message={m}
                        status={status}
                        commands={commands}
                        editing={editingId === m.id}
                        keyboardOpen={keyboardOpen}
                        onResumeKeyboardSurface={onResumeKeyboardSurface}
                        onEdit={(): void => {
                          setReordering(false);
                          setEditingId(m.id);
                        }}
                        onEditDone={(): void => {
                          const restoreFocus = !suppressEditFocusRestoreRef
                            .current;
                          suppressEditFocusRestoreRef.current = false;
                          setEditingId(null);
                          if (restoreFocus) {
                            requestAnimationFrame(() =>
                              scrollRef.current?.querySelector<HTMLElement>(
                                `[data-desktop-item="${CSS.escape(m.id)}"]`,
                              )?.focus({ preventScroll: true })
                            );
                          }
                        }}
                        onEditController={registerEditController}
                        onMove={onMoveDraft
                          ? (): void => onMoveDraft(m.id)
                          : undefined}
                        onSchedule={onScheduleDraft
                          ? (): void => onScheduleDraft(m.id)
                          : undefined}
                      />
                    )}
                </Box>
              </Stack>
            );
          })}
        </Stack>
      </Collapse>
      <ConfirmSheet
        open={confirmCollapseEdit}
        onClose={(): void => setConfirmCollapseEdit(false)}
        title="Save edits before closing?"
        actions={
          <>
            <Button
              color="inherit"
              onClick={(): void => setConfirmCollapseEdit(false)}
            >
              Keep editing
              <Kbd keys="Esc" />
            </Button>
            <Button
              color="error"
              onClick={(): void => settleEditAndCollapse("discard")}
            >
              Discard
            </Button>
            <Button
              variant="contained"
              onClick={(): void => settleEditAndCollapse("save")}
            >
              Save &amp; collapse
              <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
            </Button>
          </>
        }
      >
        <DialogContentText>
          This {kind === "queued" ? "queued message" : "draft"}{" "}
          has unsaved changes. Save or discard them before collapsing the
          panel.
        </DialogContentText>
      </ConfirmSheet>
    </Box>
  );
}

// One queued prompt. Read mode shows the (clamped) text + a primary action +
// Edit / Delete. Desktop edit mode is transactional: Mod+Enter saves and Esc
// asks before discarding. The primary action depends on whether the session can take a turn
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

function pendingContentCleared(
  text: string,
  attachments: Attachment[],
): boolean {
  return stripImageTokens(text).trim() === "" && attachments.length === 0;
}

function PendingRow({
  desktop,
  kind,
  sessionId,
  message,
  status,
  commands,
  editing,
  keyboardOpen,
  onResumeKeyboardSurface,
  onEdit,
  onEditDone,
  onEditController,
  onMove,
  onSchedule,
}: {
  desktop: boolean;
  kind: "queued" | "draft";
  sessionId: string;
  message: QueuedMessage;
  status: Status;
  commands: () => AvailableCommand[];
  editing: boolean;
  /** Panel-owned software-keyboard state, shared by every row. */
  keyboardOpen: boolean;
  onResumeKeyboardSurface?: (() => void) | undefined;
  onEdit: () => void;
  onEditDone: () => void;
  onEditController: (controller: PendingEditController | null) => void;
  /** Open the move-to-another-session picker for this row (draft kind only). */
  onMove?: (() => void) | undefined;
  /** Open the schedule picker for this row (draft kind only) — set/reschedule
   *  its future auto-send. Absent → the row omits the schedule chip + action. */
  onSchedule?: (() => void) | undefined;
}): React.JSX.Element {
  const initialEditText = useRef(
    promoteUnplacedImageTokens(message.text, message.attachments),
  ).current;
  const [draft, setDraft] = useState(initialEditText);
  const editTextRef = useRef(initialEditText);
  const [committedText, setCommittedText] = useState<string | null>(null);
  const [committedAttachments, setCommittedAttachments] = useState<
    Attachment[] | null
  >(null);
  const seedText = pendingRowVisibleText(message.text, committedText);
  const seedAttachments = committedAttachments ?? message.attachments;
  const seedTray = attachmentTrayForSurface(seedAttachments, seedText);
  // Per-row kebab (⋮) position. Click coordinates, not the button node: the
  // peek/footer keep a translate3d layer, and hiding the kebab via container
  // query can detach the element so MUI's anchorEl lands at (0, 0).
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(
    null,
  );
  const suppressEditTapUntilRef = useRef(0);
  const pendingEditDirtyRef = useRef(false);
  const suppressPendingEditTap = (): void => {
    suppressEditTapUntilRef.current = performance.now() + 480;
  };
  // Local attachments while editing, seeded from the queued message. The edit
  // box is the SAME ComposerEditor as the main composer, so a queued prompt can
  // gain/lose images here too (pasted screenshots, picked files).
  const [editAttachments, setEditAttachments] = useState<Attachment[]>(() => {
    // Seed the inline-image registry so this message's `cowboy-att:` tokens render
    // as thumbnails in the edit box (the same ComposerEditor + plugin), including
    // queued items synced from another terminal.
    seedInlineAttachments(message.attachments);
    return message.attachments;
  });
  const editAttachmentsPending = editAttachments.some((attachment) =>
    attachment.pending
  );
  const updateEditDraft = (next: string): void => {
    const previous = editTextRef.current;
    editTextRef.current = next;
    pendingEditDirtyRef.current = true;
    setEditAttachments((current) =>
      reconcileDeletedInlineImages(previous, next, current, getInlineAttachment)
    );
    setDraft(next);
  };
  const editorRef = useRef<ComposerEditorHandle>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  const connected = useConnected();
  // Confirm popover for force push (anchored to the Bolt button). Null = closed.
  const [confirmAnchor, setConfirmAnchor] = useState<HTMLElement | null>(null);
  // Per-row delete confirm. Dropping a queued message / draft is irreversible, so
  // the × opens this modal instead of deleting on a single tap.
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [confirmDiscardEdit, setConfirmDiscardEdit] = useState(false);
  const doRemove = (): void => {
    importantHaptic();
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
  // Focused edit overlay (Zed-style ↗): the FullscreenComposer — the SAME expand
  // component the main input uses — so a long queued message edits comfortably
  // (full toolbar + inline images) without ballooning the queue panel inline. ONE
  // editor is mounted at a time (inline XOR overlay), both driving the shared
  // `draft` — so there's no uncontrolled-editor desync.
  // Same vim setting as the composer — editing a queued/draft message uses the same
  // editor surface, so it gets vim too.
  const vim = useVimSetting();
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [mobileToolbarSettingsOpen, setMobileToolbarSettingsOpen] = useState(
    false,
  );
  const [hasEditSelection, setHasEditSelection] = useState(false);
  const mobileToolbarIds = useComposerToolbar();
  const overlayEditorRef = useRef<ComposerEditorHandle>(null);
  const editFileInputRef = useRef<HTMLInputElement>(null);
  const beginEdit = (): void => {
    if (performance.now() < suppressEditTapUntilRef.current) return;
    pendingEditDirtyRef.current = false;
    // iOS will only open its software keyboard when the real editable control is
    // focused inside the originating tap. React normally batches `onEdit()` and
    // mounts the row editor after that activation window, leaving a visible edit
    // card with no keyboard. Commit the edit surface synchronously, then hand the
    // same gesture directly to it. This also arms UIKit's native paste/select
    // recognizer; a hidden keyboard claim or rAF focus cannot do that.
    if (touchInput) {
      // The originating tap is the same "fresh editor interaction" the primary
      // card uses to clear the compact latch. A transient visualViewport close
      // during the focus handoff must not re-latch Hide/delivery and leave this
      // row stuck as "N Drafts Editing" after the keyboard is already up.
      beginMobileEditorFocusTransfer();
      onResumeKeyboardSurface?.();
      flushSync(() => {
        setDraft(seedText);
        editTextRef.current = seedText;
        setEditAttachments(seedAttachments);
        onEdit();
      });
      editorRef.current?.focusEnd();
      return;
    }
    setDraft(seedText);
    editTextRef.current = seedText;
    setEditAttachments(seedAttachments);
    onEdit();
  };
  const pendingEditTap = useReliableTouchTap<HTMLDivElement>(() => {
    beginEdit();
  });
  const discardEdit = (): void => {
    setConfirmDiscardEdit(false);
    setDraft(seedText);
    editTextRef.current = seedText;
    setEditAttachments(seedAttachments);
    setOverlayOpen(false);
    onEditDone();
  };
  const liveEditText = (): string =>
    pendingEditLiveText(
      overlayEditorRef.current?.getValue() ?? editorRef.current?.getValue(),
      editTextRef.current,
    );
  const persistEdit = (): boolean => {
    // A synchronous paste placeholder has no ACP bytes yet. Keep the editor
    // alive until encoding either promotes or removes it; persisting the empty
    // block would make the pending row restore a broken inline token.
    if (editAttachmentsPending) return false;
    const text = liveEditText();
    editTextRef.current = text;
    setDraft(text);
    if (pendingContentCleared(text, editAttachments)) {
      if (kind === "draft") removeDraft(sessionId, message.id);
      else removeQueued(sessionId, message.id);
      setOverlayOpen(false);
      onEditDone();
      return false;
    }
    if (kind === "draft") {
      editDraft(sessionId, message.id, text, editAttachments);
    } else editQueued(sessionId, message.id, text, editAttachments);
    setCommittedText(text);
    setCommittedAttachments(editAttachments);
    return true;
  };
  useEffect(() => {
    if (!editing || !pendingEditDirtyRef.current || editAttachmentsPending) {
      return;
    }
    if (
      !pendingContentCleared(
        pendingEditLiveText(
          overlayEditorRef.current?.getValue() ?? editorRef.current?.getValue(),
          editTextRef.current,
        ),
        editAttachments,
      )
    ) return;
    if (kind === "draft") removeDraft(sessionId, message.id);
    else removeQueued(sessionId, message.id);
    setOverlayOpen(false);
    onEditDone();
  }, [
    draft,
    editAttachments,
    editAttachmentsPending,
    editing,
    kind,
    message.id,
    onEditDone,
    sessionId,
  ]);
  const saveEdit = (): void => {
    if (!persistEdit()) return;
    setOverlayOpen(false);
    onEditDone();
  };
  const completePendingDelivery = async (
    operation: () => Promise<void>,
  ): Promise<void> => {
    if (!persistEdit()) return;
    await operation();
    setOverlayOpen(false);
    onEditDone();
  };
  const sendDraftFromEdit = (): Promise<void> =>
    kind === "draft"
      ? completePendingDelivery(() => activateDraft(sessionId, message.id))
      : Promise.resolve();
  const scheduleDraftFromEdit = (): void => {
    if (kind !== "draft" || onSchedule === undefined) return;
    // The schedule sheet targets the durable draft id. Persist the current
    // inline buffer before opening it so scheduling never sends stale text.
    if (!persistEdit()) return;
    setOverlayOpen(false);
    onEditDone();
    onSchedule();
  };
  const confirmForcePush = async (): Promise<void> => {
    // `starting` has no interruptible turn yet, and a disconnected client would
    // drop this non-durable command. Keep the event path guarded as well as the
    // button so keyboard/confirm callbacks cannot bypass the disabled state.
    if (status !== "busy") return;
    if (editing) {
      await completePendingDelivery(() =>
        forcePushQueued(sessionId, message.id)
      );
    } else {
      await forcePushQueued(sessionId, message.id);
    }
    setConfirmAnchor(null);
  };
  useConfirmEnter(confirmAnchor !== null, () => {
    void confirmForcePush();
  });
  const forcePushConfirmation = kind === "queued"
    ? (
      <Popper
        open={confirmAnchor !== null}
        anchorEl={confirmAnchor}
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
        <ClickAwayListener onClickAway={(): void => setConfirmAnchor(null)}>
          <Paper
            role="dialog"
            aria-modal="false"
            aria-label="Force push confirmation"
            sx={{ maxWidth: 240, borderRadius: "12px", boxShadow: 8 }}
          >
            <Box sx={{ p: 1.5 }}>
              <Typography variant="body2" sx={{ mb: 1 }}>
                Stop the current turn and send this message now? The agent's
                in-progress work is discarded.
              </Typography>
              <Stack direction="row" spacing={1} justifyContent="flex-end">
                <Button
                  size="small"
                  color="inherit"
                  onPointerDown={(event): void => event.preventDefault()}
                  onClick={(): void => setConfirmAnchor(null)}
                  sx={{ textTransform: "none" }}
                >
                  Cancel
                  <Kbd keys="Esc" />
                </Button>
                <NetworkButton
                  size="small"
                  variant="contained"
                  color="warning"
                  startIcon={<Bolt />}
                  onPointerDown={(event): void => event.preventDefault()}
                  networkAction={confirmForcePush}
                  sx={{ textTransform: "none" }}
                >
                  Force push
                  <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
                </NetworkButton>
              </Stack>
            </Box>
          </Paper>
        </ClickAwayListener>
      </Popper>
    )
    : null;
  // Mobile Queue/Draft edits own the writing surface while the keyboard is
  // up. Dismissing it must persist the live buffer, then return to the
  // compact card — never leave the two-track dock over an empty canvas.
  const mobileEditSawKeyboardRef = useRef(false);
  const persistEditRef = useRef(persistEdit);
  persistEditRef.current = persistEdit;
  const finishMobileEdit = (): void => {
    if (!touchInput || editAttachmentsPending) return;
    persistEditRef.current();
    setOverlayOpen(false);
    onEditDone();
    dismissMobileSoftwareKeyboard();
    releaseMobileComposerFocus();
  };
  const finishMobileEditRef = useRef(finishMobileEdit);
  finishMobileEditRef.current = finishMobileEdit;
  const hideMobileEditKeyboard = (): void => {
    finishMobileEdit();
  };
  useEffect(() => {
    if (!touchInput || !editing) {
      mobileEditSawKeyboardRef.current = false;
      return undefined;
    }
    if (keyboardOpen) {
      mobileEditSawKeyboardRef.current = true;
      return undefined;
    }
    if (editAttachmentsPending) return undefined;
    // Entering an edit and raising a third-party keyboard are not atomic.
    // Persist after one animation window, but keep the editor mounted so a
    // late IME frame cannot delete the row or reveal the empty composer.
    if (!mobileEditSawKeyboardRef.current) {
      const timer = globalThis.setTimeout(
        () => persistEditRef.current(),
        700,
      );
      return () => globalThis.clearTimeout(timer);
    }
    // A native long press can transiently report a closed keyboard while UIKit
    // promotes the textarea gesture into Paste/Select. After that settle
    // window, leave the two-track chrome and restore the compact card.
    const timer = globalThis.setTimeout(
      () => finishMobileEditRef.current(),
      mobilePendingKeyboardCloseSettleMs,
    );
    return () => globalThis.clearTimeout(timer);
  }, [editAttachmentsPending, editing, keyboardOpen, touchInput]);
  const keyboardBoundEditing = editing;
  const editDirty = draft !== seedText ||
    editAttachments.length !== seedAttachments.length ||
    editAttachments.some((attachment, index) =>
      attachment.id !== seedAttachments[index]?.id
    );
  // Register one stable controller for the lifetime of the active edit. Its
  // methods read this ref, so the panel header always resolves the latest text
  // and attachments without turning every keystroke into parent React state.
  const latestEditTransactionRef = useRef<
    {
      dirty: boolean;
      save: () => void;
      discard: () => void;
    } | null
  >(null);
  latestEditTransactionRef.current = {
    dirty: editDirty,
    save: saveEdit,
    discard: discardEdit,
  };
  useLayoutEffect(() => {
    if (!editing) return undefined;
    const controller: PendingEditController = {
      isDirty: () => latestEditTransactionRef.current?.dirty ?? true,
      save: () => latestEditTransactionRef.current?.save(),
      discard: () => latestEditTransactionRef.current?.discard(),
    };
    onEditController(controller);
    return () => onEditController(null);
  }, [editing, onEditController]);
  // Open after the initiating Escape has finished bubbling. Mounting MUI's
  // Dialog synchronously inside that same key event lets its escape listener
  // observe the trigger and immediately close the brand-new confirmation.
  const requestDiscardEdit = (): void => {
    globalThis.requestAnimationFrame(() => setConfirmDiscardEdit(true));
  };
  useConfirmEnter(confirmDiscardEdit, discardEdit);
  useLayoutEffect(() => {
    // Touch opens through the accessory tap below, which snapshots selection,
    // flush-mounts the overlay, and transfers focus once inside that gesture.
    // Re-focusing here would overwrite the restored caret and can make WebKit
    // decline the already-open software keyboard. Desktop still needs one frame
    // for its lazy CM6/Vim mount.
    if (!overlayOpen || touchInput) return undefined;
    const frame = globalThis.requestAnimationFrame(() =>
      overlayEditorRef.current?.focusEnd()
    );
    return () => globalThis.cancelAnimationFrame(frame);
  }, [overlayOpen, touchInput]);
  useLayoutEffect(() => {
    if (!editing) return undefined;
    // Queue/Draft edits begin in the compact card on every surface. Fullscreen
    // remains an explicit action for long-form work rather than an automatic
    // navigation caused by a small Edit tap.
    // focusEnd, not focus: opening an existing draft/queued message should put
    // the caret at the end of its text so you continue typing, not at the start.
    // Desktop focus belongs to PlatformComposerEditor's final interactive mount.
    // Its Vim runtime can resolve after this frame and replace the loading CM6
    // instance, so focusing here would hand keyboard ownership to stale DOM.
    // Touch entry already mounted and focused the real textarea synchronously in
    // `beginEdit`. Re-focusing on the next frame falls outside the user gesture
    // and can make WebKit keep the caret while declining to show the keyboard.
    if (touchInput) {
      rowRef.current?.scrollIntoView({ block: "center", behavior: "auto" });
      return undefined;
    }
    const frame = globalThis.requestAnimationFrame(() => {
      rowRef.current?.scrollIntoView({ block: "center", behavior: "auto" });
    });
    return () => globalThis.cancelAnimationFrame(frame);
  }, [editing, touchInput]);
  if (keyboardBoundEditing) {
    const editSendable = (!!draft.trim() || editAttachments.length > 0) &&
      !editAttachmentsPending;
    const editTrayAttachments = attachmentTrayForSurface(
      editAttachments,
      draft,
    );
    const activeEditEditor = (): ComposerEditorHandle | null =>
      overlayEditorRef.current ?? editorRef.current;
    const addEditFiles = (
      files: File[],
      options: {
        preserveFocus?: boolean;
        selection?: ComposerEditorSelection;
      } = {},
    ): void => {
      if (files.length === 0) return;
      if (options.preserveFocus) {
        const images = files.filter((file) =>
          (file.type || "").startsWith("image/")
        );
        const rest = files.filter((file) =>
          !(file.type || "").startsWith("image/")
        );
        if (images.length > 0) {
          // Match the primary Composer's iOS paste path: place every image and
          // its caret synchronously while the native Paste gesture is live, then
          // replace the object-URL placeholders with durable ACP bytes. Waiting
          // for raster encoding first unmounts the textarea outside UIKit's
          // activation window and loses the keyboard/selection.
          const pending = images.map((file) => pendingImageAttachment(file));
          pending.forEach(registerInlineAttachment);
          setEditAttachments((previous) => [...previous, ...pending]);
          activeEditEditor()?.insertImages(pending, options.selection);
          void Promise.allSettled(
            images.map((file, index) =>
              fileToAttachment(file, pending[index]!.id)
            ),
          ).then((settled) => {
            const completed = settled.flatMap((result) =>
              result.status === "fulfilled" ? [result.value] : []
            );
            completed.forEach(registerInlineAttachment);
            const completedById = new Map(
              completed.map((item) => [item.id, item]),
            );
            const failed = pending.filter((item) =>
              !completedById.has(item.id)
            );
            failed.forEach((item) => activeEditEditor()?.deleteImage(item.id));
            setEditAttachments((current) =>
              settlePendingAttachments(current, pending, completed)
            );
            activeEditEditor()?.refreshImages();
            pending.forEach((item) => {
              if (item.previewUrl?.startsWith("blob:")) {
                URL.revokeObjectURL(item.previewUrl);
              }
            });
          });
        }
        if (rest.length === 0) return;
        files = rest;
      }
      void filesToAttachments(files).then((added) => {
        if (added.length === 0) return;
        // Mirror addFiles: register bytes BEFORE inserting the token so the
        // inline decoration resolves synchronously, keep editAttachments as the
        // send source, then drop an inline `cowboy-att:` token at the caret.
        // Without the register+insert, a pasted image during edit only became a
        // footer chip and never rendered inline in the editor (the reported bug).
        added.forEach(registerInlineAttachment);
        setEditAttachments((prev) => [...prev, ...added]);
        // Insert the token into whichever editor is actually MOUNTED: the
        // fullscreen overlay owns the edit while open (overlayEditorRef), and the
        // inline editor is unmounted then (`{!overlayOpen && …}`). Hardcoding
        // editorRef dropped the token when pasting in the expanded/overlay editor
        // — insertImage no-op'd on the null inline ref, so the image became a
        // gallery-only chip with no inline token (collapsed showed it, expanded
        // didn't — the reported "展开末尾粘贴图片还是有 bug").
        activeEditEditor()?.insertImages(added);
      });
    };
    const pasteEditClipboardImages = (
      request: NativeClipboardImagePasteRequest,
    ): Promise<void> =>
      runNativeClipboardImagePaste(request, {
        stage: (pending, selection): void => {
          pending.forEach(registerInlineAttachment);
          const editor = activeEditEditor();
          flushSync(() => {
            setEditAttachments((previous) => [...previous, ...pending]);
            editor?.insertImages(pending, selection);
          });
        },
        settle: (pending, completed): void => {
          completed.forEach(registerInlineAttachment);
          const completedById = new Map(
            completed.map((attachment) => [attachment.id, attachment]),
          );
          const editor = activeEditEditor();
          pending.filter((attachment) => !completedById.has(attachment.id))
            .forEach((attachment) => editor?.deleteImage(attachment.id));
          setEditAttachments((current) =>
            settlePendingAttachments(current, pending, completed)
          );
          editor?.refreshImages();
        },
      });
    // Editing reuses the composer surface, but Desktop treats it as a transaction:
    // Mod+Enter commits and Esc asks before throwing away the local buffer.
    const desktopEditBar = (
      <ComposeBar
        desktop={desktop}
        dead={false}
        sendable={editSendable}
        attachments={editAttachments}
        onRemoveAttachment={(id): void =>
          setEditAttachments((prev) => prev.filter((a) => a.id !== id))}
        onTrigger={(t): void => editorRef.current?.insertTrigger(t)}
        onAttach={(): void => editFileInputRef.current?.click()}
        onSend={saveEdit}
        submitLabel="Done editing"
        submitIcon={<Check />}
        onExpand={(): void => setOverlayOpen(true)}
      />
    );
    const expandMobileEdit = (): void => {
      haptic();
      const selection = editorRef.current?.getSelection();
      beginMobileEditorFocusTransfer();
      flushSync(() => setOverlayOpen(true));
      if (selection) overlayEditorRef.current?.focusSelection(selection);
      else overlayEditorRef.current?.focusEnd();
    };
    const editFormatActions = (
      <MobileComposerFormatActions
        commandIds={hasEditSelection
          ? ["undo", "redo", "bold", "italic", "code", "link"]
          : mobileToolbarIds}
        editorRef={editorRef}
        onAttach={(): void => editFileInputRef.current?.click()}
        onPasteImages={pasteEditClipboardImages}
        onCustomize={(): void => {
          releaseMobileComposerFocus();
          setMobileToolbarSettingsOpen(true);
        }}
      />
    );
    return (
      <>
        {/* One file input for BOTH the inline edit and the overlay's attach button. */}
        <input
          ref={editFileInputRef}
          type="file"
          accept="image/*,text/*,application/pdf,.md,.json,.csv,.log"
          multiple
          hidden
          onChange={(e): void => {
            addEditFiles(Array.from(e.target.files ?? []));
            e.target.value = "";
          }}
        />
        <Paper
          ref={rowRef}
          variant="outlined"
          tabIndex={desktop ? -1 : undefined}
          data-mobile-focus-composer={touchInput ? "true" : undefined}
          data-mobile-pending-editor={touchInput ? "true" : undefined}
          data-mobile-keyboard-open={touchInput && keyboardOpen
            ? "true"
            : undefined}
          sx={{
            position: "relative",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
            ...(desktop ? { p: 0.75 } : {
              borderRadius: mobileComposerPanelFrameSx.borderRadius,
              bgcolor: "transparent",
              borderColor: (theme) => alpha(theme.palette.primary.main, 0.42),
              transition:
                `border-color ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, background-color ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}, box-shadow ${mobileComposerFocusMotion.duration} ${mobileComposerFocusMotion.easing}`,
              // Match the primary card: visualViewport is the keyboard authority.
              // Do not wait for :focus-within — iOS can move DOM focus to body
              // while the keyboard and caret stay up.
              "&[data-mobile-keyboard-open='true']": {
                ...mobileFocusedComposerSurfaceSx,
                backgroundImage: "none",
              },
              "&[data-mobile-keyboard-open='true'] [data-mobile-editor-area]": {
                bgcolor: mobileFocusedComposerFill,
                backgroundImage: "none",
                flex: "0 1 auto",
                maxHeight: "min(42dvh, 22rem)",
                overflow: "hidden",
              },
              "&[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-theme-none, &[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-editor, &[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-scroller, &[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-content":
                {
                  bgcolor: mobileFocusedComposerFill,
                  backgroundImage: "none",
                  WebkitOverflowScrolling: "auto",
                },
              "&[data-mobile-keyboard-open='true'] [data-mobile-editor-area] .cm-scroller":
                {
                  flex: "0 1 auto",
                  minHeight: 0,
                  height: "auto",
                  maxHeight: "min(42dvh, 22rem)",
                  overflowX: "hidden",
                  overflowY: "auto",
                  scrollPaddingBlock: "12px",
                  overscrollBehavior: "contain",
                  touchAction: "pan-y pinch-zoom",
                },
              "& [data-mobile-composer-accessory]": {
                flexShrink: 0,
              },
            }),
          }}
          onKeyDownCapture={desktop
            ? (event): void => {
              if (
                event.key !== "Escape" ||
                isImeKeyEvent(event.nativeEvent)
              ) return;
              // Capture is required because Normal-mode focus lives on the
              // non-editable Vim command sink. It must nevertheless respect
              // the editor's actual mode: Insert/Visual/operator Escape belongs
              // to Vim first; only plain Normal may open the discard dialog.
              if (!editorRef.current?.escapeBelongsToApp()) return;
              event.preventDefault();
              event.stopPropagation();
              requestDiscardEdit();
            }
            : undefined}
        >
          {desktop && !overlayOpen && (
            <Suspense fallback={null}>
              <DesktopPendingEditCommandBindings
                kind={kind}
                sendable={editSendable}
                onSlash={(): void => editorRef.current?.insertTrigger("/")}
                onReference={(): void => editorRef.current?.insertTrigger("@")}
                onAttach={(): void => editFileInputRef.current?.click()}
                onDone={saveEdit}
                onExpand={(): void => setOverlayOpen(true)}
              />
            </Suspense>
          )}
          {
            /* Inline editor — hidden while the focused overlay owns the edit so only
              ONE editor is mounted at a time (shared `draft`, no uncontrolled desync). */
          }
          {!overlayOpen && (
            <Box
              data-mobile-editor-area={touchInput ? "true" : undefined}
              onPointerDown={touchInput
                ? (): void => {
                  onResumeKeyboardSurface?.();
                }
                : undefined}
              sx={{
                display: "flex",
                flexDirection: "column",
                minHeight: 0,
              }}
            >
              {touchInput && editTrayAttachments.length > 0 && (
                <AttachmentPreviews
                  attachments={editTrayAttachments}
                  onRemove={(id): void =>
                    setEditAttachments((previous) =>
                      previous.filter((attachment) => attachment.id !== id)
                    )}
                />
              )}
              <PlatformComposerEditor
                ref={editorRef}
                // Seeds from the shared `draft` and re-mounts on overlay close, so
                // it reflects edits made in the overlay. Desktop CM6 remains
                // uncontrolled thereafter; touch uses the controlled native editor
                // until an inline image token requires CM6 (vim desktop-only).
                value={draft}
                borderless
                {...(touchInput ? { endInset: 36 } : {})}
                vim={touchInput ? false : vim}
                focusEndOnMount={desktop}
                onVimMode={setVimMode}
                onChange={updateEditDraft}
                onSubmit={saveEdit}
                sessionId={sessionId}
                commands={commands}
                placeholder="Edit message…"
                onPasteFiles={(files): void =>
                  addEditFiles(files, { preserveFocus: true })}
                onSelectionChange={setHasEditSelection}
                onEscape={(): boolean => {
                  if (desktop) requestDiscardEdit();
                  else hideMobileEditKeyboard();
                  return true;
                }}
              />
            </Box>
          )}
          {!overlayOpen &&
            (desktop ? desktopEditBar : (
              <MobileComposerAccessoryDock
                embedded
                mode={hasEditSelection ? "selection" : "insert"}
                formatActions={editFormatActions}
                utilityActions={
                  <>
                    <MobileComposerAccessoryButton
                      title="Attach file"
                      onClick={(): void => editFileInputRef.current?.click()}
                    >
                      <AttachFile />
                    </MobileComposerAccessoryButton>
                    {kind === "draft" && onSchedule && (
                      <MobileComposerAccessoryButton
                        title={message.schedule
                          ? "Reschedule send"
                          : "Schedule send"}
                        disabled={!editSendable}
                        onClick={(): void => {
                          haptic();
                          scheduleDraftFromEdit();
                        }}
                      >
                        <Schedule />
                      </MobileComposerAccessoryButton>
                    )}
                    {kind === "queued" && (
                      <MobileComposerAccessoryButton
                        title="Force push"
                        color="warning"
                        disabled={!editSendable || status !== "busy"}
                        onClick={(event): void => {
                          haptic();
                          setConfirmAnchor(event.currentTarget);
                        }}
                      >
                        <Bolt />
                      </MobileComposerAccessoryButton>
                    )}
                    {kind === "draft" && (
                      <MobileComposerAccessoryButton
                        title="Expand editor"
                        onClick={expandMobileEdit}
                      >
                        <OpenInFull />
                      </MobileComposerAccessoryButton>
                    )}
                  </>
                }
                fixedAction={
                  <MobileComposerAccessoryButton
                    title="Hide keyboard"
                    preserveEditorFocus={false}
                    disabled={editAttachmentsPending}
                    onClick={hideMobileEditKeyboard}
                  >
                    <KeyboardHide />
                  </MobileComposerAccessoryButton>
                }
                primaryLabel={kind === "draft" ? "Send draft" : "Expand editor"}
                primaryDisabled={kind === "draft" ? !editSendable : false}
                onPrimary={kind === "draft" ? sendDraftFromEdit : expandMobileEdit}
                primaryIcon={kind === "draft" ? <Send /> : <OpenInFull />}
              />
            ))}
        </Paper>
        {forcePushConfirmation}
        {
          /* Focused edit overlay: the row's expanded edit reuses the SAME component
            as the main input's expand — FullscreenComposer (the toolbar registry,
            inline images, native caret) — NOT a bespoke DetentSheet. Desktop keeps
            explicit Done/discard semantics. Mobile collapse leaves the compact
            row editor in place so hiding the keyboard cannot empty the buffer. */
        }
        {overlayOpen && (
          <FullscreenComposer
            editorRef={overlayEditorRef}
            value={draft}
            onChange={updateEditDraft}
            onSubmit={(): void => {
              persistEdit();
              if (touchInput) setOverlayOpen(false);
              else {
                saveEdit();
                setOverlayOpen(false);
              }
            }}
            onSaveDraft={(): void => {
              persistEdit();
              if (touchInput) setOverlayOpen(false);
              else {
                saveEdit();
                setOverlayOpen(false);
              }
            }}
            onCollapse={(): void => {
              persistEdit();
              setOverlayOpen(false);
            }}
            onAttach={(): void => editFileInputRef.current?.click()}
            onPasteFiles={(files, selection): void =>
              addEditFiles(
                files,
                selection === undefined
                  ? { preserveFocus: true }
                  : { preserveFocus: true, selection },
              )}
            onPasteClipboardImages={pasteEditClipboardImages}
            sessionId={sessionId}
            commands={commands}
            placeholder="Edit message…"
            sendable={editSendable}
            // The originating accessory tap owns exactly one focus transfer.
            autoFocus={false}
            focusEndOnMount={desktop}
            showCollapse={false}
            submitLabel={touchInput ? "Collapse editor" : "Done editing"}
            submitIcon={touchInput ? <CloseFullscreen /> : <Check />}
            vim={vim}
            onVimMode={setVimMode}
            onDiscard={discardEdit}
            attachmentsSlot={editTrayAttachments.length > 0
              ? (
                <AttachmentPreviews
                  attachments={editTrayAttachments}
                  onRemove={(id): void =>
                    setEditAttachments((prev) =>
                      prev.filter((a) => a.id !== id)
                    )}
                />
              )
              : undefined}
          />
        )}
        <ComposerToolbarSettings
          open={mobileToolbarSettingsOpen}
          onClose={(): void => setMobileToolbarSettingsOpen(false)}
        />
        <ConfirmSheet
          open={confirmDiscardEdit}
          onClose={(): void => setConfirmDiscardEdit(false)}
          title="Discard message edits?"
          actions={
            <>
              <Button onClick={(): void => setConfirmDiscardEdit(false)}>
                Keep editing
                <Kbd keys="Esc" />
              </Button>
              <Button color="error" onClick={discardEdit}>
                Discard changes
                <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
              </Button>
            </>
          }
        >
          <DialogContentText>
            Your unsaved changes to this{" "}
            {kind === "queued" ? "queued message" : "draft"} will be lost.
          </DialogContentText>
        </ConfirmSheet>
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
        onClick: beginEdit,
      },
      ...(onSchedule
        ? [{
          key: "schedule",
          label: message.schedule ? "改期 / 取消定时…" : "定时发送…",
          icon: <Schedule fontSize="small" />,
          onClick: onSchedule,
        }]
        : []),
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
        onClick: beginEdit,
      },
      {
        key: "remove",
        label: "Remove",
        icon: <Close fontSize="small" />,
        onClick: (): void => setConfirmRemove(true),
      },
    ];
  const secondaryShortcut: Record<
    string,
    { badge: string; description: string }
  > = {
    edit: { badge: "L", description: "L / Enter · edit focused item" },
    schedule: { badge: "T", description: "T · schedule focused item" },
    move: { badge: "M", description: "M · move focused item" },
    return: { badge: "R", description: "R · return focused item to drafts" },
    remove: { badge: "X", description: "X · remove focused item" },
  };

  // Primary action — always inline. Drafts always Send (send-or-queue); a queued
  // row Sends now when the session's free, else Force-pushes (confirm popover).
  // Built as a statement to avoid a nested ternary in the JSX.
  let primary: React.JSX.Element;
  const primaryEnabled = kind === "draft" || dispatchable ||
    status === "busy";
  if (kind === "draft") {
    primary = (
      <Tooltip title={dispatchable ? "Send" : "Add to queue"}>
        <NetworkIconButton
          data-desktop-item-action="default"
          size="small"
          color="primary"
          aria-label="send draft"
          networkAction={() => activateDraft(sessionId, message.id)}
        >
          <Send fontSize="small" />
        </NetworkIconButton>
      </Tooltip>
    );
  } else if (dispatchable) {
    primary = (
      <Tooltip title="Send now">
        <NetworkIconButton
          data-desktop-item-action="default"
          size="small"
          color="primary"
          aria-label="send now"
          networkAction={() => requestSendQueued(sessionId, message.id)}
        >
          <Send fontSize="small" />
        </NetworkIconButton>
      </Tooltip>
    );
  } else {
    const canForcePush = status === "busy";
    const forcePushTitle = status === "starting"
      ? "Agent is starting — available when ready"
      : !connected
      ? "Force push when back online"
      : "Force push (interrupt & send)";
    primary = (
      <Tooltip title={forcePushTitle}>
        {
          /* MUI disabled buttons do not emit pointer events; the span keeps the
            reason discoverable while the action itself remains inert. */
        }
        <span>
          <IconButton
            data-desktop-item-action="default"
            size="small"
            color="warning"
            aria-label="force push"
            disabled={!canForcePush}
            onClick={(e): void => {
              if (canForcePush) setConfirmAnchor(e.currentTarget);
            }}
          >
            <Bolt fontSize="small" />
          </IconButton>
        </span>
      </Tooltip>
    );
  }
  if (desktop) {
    const primaryControl = primary;
    primary = (
      <Suspense fallback={primaryControl}>
        <DesktopContextShortcut
          badge="S"
          shortcut="S · send focused item"
          itemScoped
          enabled={primaryEnabled}
          placement="corner"
        >
          {primaryControl}
        </DesktopContextShortcut>
      </Suspense>
    );
  }
  return (
    <Paper
      variant="outlined"
      sx={{
        p: 0.75,
        display: "flex",
        alignItems: "flex-start",
        minHeight: mobilePendingRowMinHeight,
        gap: 0.5,
        userSelect: "none",
        WebkitUserSelect: "none",
        caretColor: "transparent",
      }}
    >
      <Box
        data-pending-edit-target
        {...pendingEditTap}
        onClick={(event): void => {
          const target = event.target instanceof Element ? event.target : null;
          if (
            target?.closest(
              "button, a, input, select, textarea, [role='button'], [data-pending-content-action]",
            )
          ) return;
          pendingEditTap.onClick(event);
        }}
        sx={{
          flex: 1,
          alignSelf: "stretch",
          minWidth: 0,
          minHeight: mobileComposerIdleEditorMinHeight,
          cursor: touchInput ? "pointer" : "text",
        }}
      >
        {seedText.trim() !== "" && (
          <MessagePreview text={seedText} attachments={seedAttachments} />
        )}
        {seedTray.length > 0 && (
          <QueuedAttachmentChips attachments={seedTray} />
        )}
        {
          /* Scheduled-draft badge: a calm info chip showing when it auto-fires;
            tap to reschedule/cancel. Info (blue), never accent/red — a pending
            schedule is a notice, not a failure (conventions/ui.md §4). */
        }
        {message.schedule && (
          <Chip
            data-pending-content-action="schedule"
            size="small"
            clickable
            icon={<Schedule sx={{ fontSize: 15 }} />}
            label={`${fireLabel(message.schedule.fire_at_ms)} · ${
              message.schedule.delivery === "front" ? "队首" : "队尾"
            }`}
            onClick={onSchedule}
            sx={{
              mt: 0.75,
              height: 24,
              borderRadius: 999,
              fontSize: 12,
              fontWeight: 600,
              color: "info.main",
              bgcolor: (t) => alpha(t.palette.info.main, 0.14),
              "& .MuiChip-label": { px: 0.875 },
              "& .MuiChip-icon": { ml: 0.75, mr: -0.25, color: "info.main" },
              "&:hover": { bgcolor: (t) => alpha(t.palette.info.main, 0.22) },
            }}
          />
        )}
      </Box>
      <Stack
        direction="row"
        alignItems="center"
        data-pending-content-action="row-actions"
        sx={{ flexShrink: 0 }}
      >
        {primary}
        {/* Force-push confirm — only the queued row has the force path. */}
        {forcePushConfirmation}
        {/* Secondary actions — inline on a roomy (iPad/desktop) panel … */}
        <Stack
          direction="row"
          alignItems="center"
          sx={{ display: "none", [ROW_ACTIONS_INLINE]: { display: "flex" } }}
        >
          {secondary.map((a) => (
            <Tooltip key={a.key} title={a.label}>
              <Box component="span" sx={{ display: "inline-flex" }}>
                {desktop
                  ? (
                    <Suspense
                      fallback={
                        <IconButton
                          size="small"
                          aria-label={a.label}
                          data-desktop-item-action={a.key}
                          onClick={a.onClick}
                        >
                          {a.icon}
                        </IconButton>
                      }
                    >
                      <DesktopContextShortcut
                        badge={secondaryShortcut[a.key]?.badge ?? ""}
                        shortcut={secondaryShortcut[a.key]?.description ??
                          a.label}
                        itemScoped
                        placement="corner"
                      >
                        <IconButton
                          size="small"
                          aria-label={a.label}
                          data-desktop-item-action={a.key}
                          onClick={a.onClick}
                        >
                          {a.icon}
                        </IconButton>
                      </DesktopContextShortcut>
                    </Suspense>
                  )
                  : (
                    <IconButton
                      size="small"
                      aria-label={a.label}
                      data-desktop-item-action={a.key}
                      onClick={a.onClick}
                    >
                      {a.icon}
                    </IconButton>
                  )}
              </Box>
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
              onClick={(e): void => {
                const rect = e.currentTarget.getBoundingClientRect();
                suppressPendingEditTap();
                setMenuPos({
                  top: rect.height > 0 ? rect.bottom : e.clientY,
                  left: rect.width > 0 ? rect.right : e.clientX,
                });
              }}
            >
              <MoreVert fontSize="small" />
            </IconButton>
          </Tooltip>
        </Box>
        <Menu
          open={menuPos !== null}
          onClose={(): void => {
            suppressPendingEditTap();
            setMenuPos(null);
          }}
          anchorReference="anchorPosition"
          anchorPosition={menuPos ?? { top: 0, left: 0 }}
          transformOrigin={{ vertical: "top", horizontal: "right" }}
          disableScrollLock
        >
          {secondary.map((a) => (
            <MenuItem
              key={a.key}
              onClick={(): void => {
                suppressPendingEditTap();
                setMenuPos(null);
                a.onClick();
              }}
            >
              <ListItemIcon>{a.icon}</ListItemIcon>
              <ListItemText>{a.label}</ListItemText>
            </MenuItem>
          ))}
        </Menu>
      </Stack>
      <ConfirmSheet
        open={confirmRemove}
        onClose={(): void => setConfirmRemove(false)}
        title={kind === "draft"
          ? "Delete this draft?"
          : "Delete this queued message?"}
        actions={
          <>
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
              onClick={doRemove}
              sx={{ textTransform: "none" }}
            >
              Delete
              <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
            </Button>
          </>
        }
      >
        {message.text
          ? (
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
              {stripImageTokens(message.text)}
            </DialogContentText>
          )
          : null}
      </ConfirmSheet>
    </Paper>
  );
}

// Confirm before stopping a running turn — the in-flight turn ends, so a single
// click/Esc must not trigger it. Shared by the navbar's Stop button (SessionControls)
// and the composer editor's Esc-to-stop; each owns its own `open` state, the markup
// is one component. Command/Ctrl+Enter confirms; Esc dismisses (Dialog default).
function StopConfirmDialog({
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
// into the Conversation header, where the state belongs; this branch retains only
// Stop so a destructive turn action stays globally visible in the top toolbar.
// It remains mounted while idle and changes only to a disabled state, keeping
// the action cluster stable as turns start and finish.
export function AutoScrollAndStop({
  sessionId,
  status,
  projection = "history",
  dense = false,
  presentation = "icons",
  desktopShortcutActive = false,
}: {
  sessionId: string;
  status: Status;
  projection?: TranscriptProjection | undefined;
  dense?: boolean;
  presentation?: "icons" | "desktop-toolbar";
  desktopShortcutActive?: boolean;
}): React.JSX.Element {
  const sticky = useSticky(sessionId);
  const exploreAtTail = useExploreAtTail(sessionId);
  const [cancelOpen, setCancelOpen] = useState(false);
  const busy = status === "busy";
  const following = projection === "history"
    ? sticky
    : sticky && exploreAtTail && busy;
  const size = dense ? "small" : "medium";
  if (presentation === "desktop-toolbar") {
    return (
      <>
        <Tooltip title={busy ? "Stop current turn" : "No turn is running"}>
          <span>
            <Button
              data-desktop-item="topbar-stop"
              data-desktop-topbar-action="stop"
              size="small"
              color="error"
              variant="text"
              startIcon={<Stop fontSize="small" />}
              disabled={!busy}
              onClick={(): void => setCancelOpen(true)}
              sx={desktopSessionActionSx({
                active: desktopShortcutActive,
                open: cancelOpen,
                minWidth: 80,
              })}
            >
              <Box component="span" sx={{ flex: 1, textAlign: "left" }}>
                Stop
              </Box>
              <ShortcutKeycap
                keyLabel="S"
                variant="global"
                accent={desktopShortcutActive || cancelOpen}
                availability={shortcutAvailability(
                  Boolean(desktopShortcutActive && busy),
                  cancelOpen,
                )}
                sx={{ flexShrink: 0, ml: 0.5 }}
              />
            </Button>
          </span>
        </Tooltip>
        <StopConfirmDialog
          open={cancelOpen}
          onClose={(): void => setCancelOpen(false)}
          onConfirm={(): void => {
            importantHaptic();
            send({ type: "cancel", session_id: sessionId });
            setCancelOpen(false);
          }}
        />
      </>
    );
  }
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
              width: 32,
              height: 32,
              "& .MuiSvgIcon-root": { fontSize: 18 },
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
                width: 32,
                height: 32,
                "& .MuiSvgIcon-root": { fontSize: 18 },
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
          send({ type: "cancel", session_id: sessionId });
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
      if (touchInput) releaseMobileComposerFocus();
      setSheetFocus(sessionSettingsFocusFromEvent(event));
      setSheetOpen(true);
    };
    document.addEventListener(OPEN_SESSION_SETTINGS_EVENT, onOpen);
    return () =>
      document.removeEventListener(OPEN_SESSION_SETTINGS_EVENT, onOpen);
  }, [touchInput]);
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
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
  const hasConfig = showSkeleton || options.length > 0;
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
      {hasConfig && (
        <Tooltip title="Options">
          <IconButton
            aria-label="options"
            disabled={providerConfigSurfaceDisabled(
              status,
              options,
              optionPresentations,
            )}
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
            <Tune />
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
          dead={dead}
          compacting={status === "busy" && timelineState.compactingTail}
          compactAction={compactAction}
          clearAction={clearAction}
          onSessionAction={runSessionAction}
          projection={projection}
          onProjectionChange={onProjectionChange}
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
  onActivate,
}: {
  preset: RunConfigPreset;
  selected: boolean;
  disabled: boolean;
  pending: boolean;
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
              label="Default"
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
        {pending && (
          <CircularProgress size={14} thickness={4.5} color="inherit" />
        )}
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
  dead,
  compacting,
  compactAction,
  clearAction,
  onSessionAction,
  projection,
  onProjectionChange,
  onSelectOption,
}: {
  open: boolean;
  focus?: SessionSettingsFocus;
  onClose: () => void;
  session: SessionMeta | undefined;
  options: ConfigOption[];
  loading: boolean;
  status: Status;
  dead: boolean;
  compacting: boolean;
  compactAction: SessionAction | null;
  clearAction: SessionAction | null;
  onSessionAction: (action: SessionAction) => Promise<void>;
  projection?: TranscriptProjection | undefined;
  onProjectionChange?: ((projection: TranscriptProjection) => void) | undefined;
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
  const pendingPreset = pendingPresetId === null
    ? undefined
    : recommendedPresets.find((preset) => preset.id === pendingPresetId);
  const activePreset = pendingPreset ?? authoritativeActivePreset;
  const [customizeAgent, setCustomizeAgent] = useState(false);
  const [sessionActionsExpanded, setSessionActionsExpanded] = useState(false);
  const [workspaceOptionsExpanded, setWorkspaceOptionsExpanded] = useState(
    false,
  );
  const [cmdConfirm, setCmdConfirm] = useState<SessionAction | null>(null);
  const [reloadConfirm, setReloadConfirm] = useState(false);
  useEffect(() => {
    if (open) {
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
  useEffect(() => {
    if (pendingPresetId === null) return;
    if (!pendingPreset) {
      setPendingPresetId(null);
      return;
    }
    if (runConfigPresetChanges(pendingPreset, options).length === 0) {
      setPendingPresetId(null);
    }
  }, [options, pendingPreset, pendingPresetId]);
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
    setReloadConfirm(false);
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
      actions={useSheetSurface
        ? (
          editingTitle
            ? (
              <MobileDecisionActions
                onCancel={cancelTitle}
                confirmLabel="Save"
                onConfirm={saveTitle}
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
      {session && (
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
          dead={dead}
          actionsExpanded={sessionActionsExpanded}
          onActionsExpandedChange={setSessionActionsExpanded}
          onSessionAction={setCmdConfirm}
          onReload={(): void => setReloadConfirm(true)}
        />
      )}
      {session && <SessionProviderSection session={session} />}
      {session && (
        <WorkspaceOptionsSection
          session={session}
          expanded={workspaceOptionsExpanded}
          onExpandedChange={setWorkspaceOptionsExpanded}
          projection={projection}
          onProjectionChange={onProjectionChange}
        />
      )}
      {(loading || options.length > 0) && (
        <>
          <Divider />
          <Box data-session-settings-agent sx={{ py: 1.5 }}>
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
                <Stack spacing={1.25} sx={{ mt: 1.25 }}>
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
                            selected={activePreset?.id === preset.id}
                            disabled={dead}
                            pending={pendingPresetId === preset.id}
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
                              for (const change of changes) {
                                if (
                                  !onSelectOption(change.configId, change.value)
                                ) {
                                  setPendingPresetId(null);
                                  return;
                                }
                              }
                              setCustomizeAgent(false);
                            }}
                          />
                        ))}
                      </Box>
                      <ButtonBase
                        aria-expanded={showAgentDetails}
                        onClick={(): void => {
                          haptic();
                          setCustomizeAgent((value) => !value);
                        }}
                        sx={{
                          minHeight: 44,
                          px: 0.5,
                          borderRadius: 1.5,
                          justifyContent: "space-between",
                          color: "text.secondary",
                        }}
                      >
                        <Typography variant="body2" sx={{ fontWeight: 650 }}>
                          Customize
                        </Typography>
                        <ExpandMore
                          fontSize="small"
                          sx={{
                            transform: showAgentDetails
                              ? "rotate(180deg)"
                              : "none",
                            transition: (theme) =>
                              theme.transitions.create("transform"),
                          }}
                        />
                      </ButtonBase>
                    </>
                  )}
                  <Collapse
                    in={showAgentDetails}
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
                </Stack>
              )}
          </Box>
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
        session={reloadConfirm ? session : null}
        onClose={(): void => setReloadConfirm(false)}
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
  dead,
  actionsExpanded,
  onActionsExpandedChange,
  onSessionAction,
  onReload,
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
  dead: boolean;
  actionsExpanded: boolean;
  onActionsExpandedChange: (expanded: boolean) => void;
  onSessionAction: (action: SessionAction) => void;
  onReload: () => void;
}): React.JSX.Element {
  // Title is editable right here — this sheet already shows the session's identity,
  // so the rename (edit-title) belongs with it rather than off in app Settings.
  // Strip the auto "provider · " prefix like the navbar does, so you edit the
  // DISPLAY title, not the machine string. The parent owns the draft because
  // mobile replaces the footer Close action with an explicit Save action while
  // this field is being edited. Plain Enter is intentionally not a commit path.
  const project = sessionProjectLabel(session) ?? "Not recorded";
  const contextUsed = session.context_used ?? 0;
  const contextSize = session.context_size ?? 0;
  const hasContext = contextSize > 0;
  const contextPercent = hasContext
    ? Math.min(100, Math.max(0, contextUsed / contextSize * 100))
    : 0;
  const cacheProtectionVisible = currentProviderEntry(
        session.provider,
        session.provider_version,
        session.provider_generation_digest,
      )?.manifest.host.features.includes("cache_protection_v1") === true &&
    contextUsed >= DEEPSEEK_CACHE_MIN_HIT_TOKENS;
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
        if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
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
    { label: "Working dir", value: session.cwd, mono: true },
    { label: "Source", value: originLabel(session.origin) },
    { label: "Status", value: session.status },
    { label: "Session id", value: session.id, mono: true },
  ];
  const cacheBaseInterval =
    compactCacheDuration(cacheProtection?.base_interval_ms) ||
    DEEPSEEK_CACHE_BASE_INTERVAL_LABEL;
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
                  ? "Cache protection off"
                  : cacheProtectionUnavailable
                  ? "Cache status unavailable"
                  : "Cache protection learning"}
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
                Reload session
              </Button>
              {compactAction && (
                <Button
                  fullWidth
                  variant="outlined"
                  startIcon={<Compress />}
                  disabled={dead || compacting}
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
                  disabled={dead}
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

function SessionProviderSection({
  session,
}: {
  session: SessionMeta;
}): React.JSX.Element {
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
              <>
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
              </>
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
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const ctrl = new AbortController();
    void fetch("/api/usage", { signal: ctrl.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
        setSnapshot(await response.json() as UsageSnapshot);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (ctrl.signal.aborted) return;
        setError(
          cause instanceof Error ? cause.message : "Could not load usage",
        );
      });
    return (): void => ctrl.abort();
  }, [provider]);
  const usage = providerUsage(
    snapshot,
    provider,
    providerVersion,
    providerDigest,
  );
  const rows = sessionProviderUsageRows(usage);
  if (error) {
    return <SheetDetailRow label="Usage" value={error} />;
  }
  if (!snapshot) {
    return <SheetDetailRow label="Usage" value="Loading…" />;
  }
  if (rows.length === 0) {
    return (
      <SheetDetailRow
        label="Usage"
        value={sessionProviderUsageEmptyMessage(usage)}
      />
    );
  }
  return (
    <>
      {rows.map((row) => (
        <Box key={row.id}>
          <SheetDetailRow label={row.label} value={row.value} />
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
          fontVariantNumeric: "tabular-nums",
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
