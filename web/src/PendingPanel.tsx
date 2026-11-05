import { desktopSize, glyphSize } from "./surface/desktopSize";
import { DeliveryPicker } from "./DeliveryPicker";
import { NativeDeliveryRow } from "./NativeDeliveryRow";
import { QueueDeliverySettings } from "./QueueDeliverySettings";
import { moveSessionDraftToDocument } from "./documents/sessionDraftImport";
import { ProtectedImage } from "./ProtectedImage";
import { lazy, memo, type ReactNode, Suspense, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { alpha, Box, Button, ButtonBase, Chip, CircularProgress, Collapse, DialogContentText, IconButton, keyframes, ListItemIcon, ListItemText, Menu, MenuItem, Paper, Stack, Tooltip, Typography, useMediaQuery, useTheme, type Theme } from "@mui/material";
import { AlternateEmail, AttachFile, Check, ChevronRight, Close, CloudUpload, CloseFullscreen, DeleteOutline, DescriptionOutlined, DragIndicator, DriveFileMoveOutlined, EditNoteOutlined, EditOutlined, ExpandMore, InsertDriveFileOutlined, KeyboardHide, MoreVert, OpenInFull, Refresh, Schedule, Send, SwapVert, Tune, Undo, VerticalAlignTop } from "@mui/icons-material";
import { type ComposerEditorHandle, type ComposerEditorSelection, PlatformComposerEditor } from "./composer/PlatformComposerEditor";
import { type NativeClipboardImagePasteRequest, runNativeClipboardImagePaste } from "./composer/nativeClipboardImagePaste";
import { dismissMobileSoftwareKeyboard, mobilePendingKeyboardCloseSettleMs, beginMobileEditorFocusTransfer, releaseMobileComposerFocus } from "./composer/mobileComposerFocus";
import { attachmentTrayForSurface } from "./composer/attachmentPresentation";
import { canReturnFromPendingRow, homeForOrigin, pendingSyncAppearance, returnLabelForHome } from "./localFirstDelivery";
import { flushSync } from "react-dom";
import { FullscreenComposer } from "./FullscreenComposer";
import { ComposerToolbarSettings } from "./ComposerToolbarSettings";
import { useComposerToolbar } from "./composerToolbarConfig";
import { MobileComposerFormatActions } from "./MobileComposerFormatActions";
import { MobileComposerAccessoryButton, MobileComposerAccessoryDock } from "./MobileComposerAccessoryDock";
import { MessagePreview } from "./MessagePreview";
import { CallNoticeLine, pendingCallNotice } from "./CallNoticeLine";
import { QueuedAttachmentChips } from "./QueuedAttachmentChips";
import { useTouchComposer } from "./ComposerTextarea";
import { pendingEditLiveText, pendingPanelDisclosureDecision, pendingRowVisibleText } from "./pendingEditLifecycle";
import { clearPendingEdit, getPendingEdit, recoverPendingEditId, subscribePendingEditRestore, setPendingEdit } from "./pendingEditStore";
import { isImeKeyEvent } from "./imeKey";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { ShortcutKeycap } from "./ShortcutKeycap";
import { HintTooltip } from "./HintTooltip";
import { openLightbox } from "./ResourceLightbox";
import { mobileComposerFocusMotion, mobileComposerIdleEditorMinHeight, mobileComposerPanelFrameSx, mobileComposerPanelHeaderMinHeight, mobilePendingRowMinHeight } from "./mobileComposerPrimitives";
import { mobileComposerOutlineColor, mobileFocusedComposerFill, mobileFocusedComposerSurfaceSx } from "./mobileComposerSurface";
import { setVimMode } from "./vimModeStore";
import { useVimSetting } from "./vimSetting";
import { DESKTOP_SHORTCUTS, DESKTOP_WORKSPACE_KEYS, desktopWorkspaceSequence } from "./desktop/commands/workspaceShortcuts";
import { type Attachment, filesToAttachments, fileToAttachment, pendingImageAttachment, imageTokensInText, promoteUnplacedImageTokens, reconcileDeletedInlineImages, settlePendingAttachments, stripImageTokens } from "./attachments";
import { getInlineAttachment, registerInlineAttachment, seedInlineAttachments } from "./inlineImages";
import { cancelTurn, currentTurnPrompt, activateAllDrafts, activateDraft, clearDrafts, clearQueue, discardQueued, returnFailedQueued, editDraft, editQueued, type QueuedMessage, notify, queuedToDraft, removeDraft, removeQueued, reorderDrafts, reorderQueue, requestSendQueued, retryQueued, setQueueEditing, useConnected, useStoreSelector } from "./store";
import { fireLabel } from "./scheduleTime";
import { haptic, importantHaptic, navigationHaptic } from "./haptic";
import { useSortable } from "./useSortable";
import { useReliableTouchTap } from "./useReliableTouchTap";
import { NetworkButton, NetworkIconButton } from "./NetworkActionFeedback";
import type { AvailableCommand, ConfigOption, Status } from "./protocol";
import { ConfirmSheet } from "./Sheet";
import { useStore as usePrefStore } from "@cowboy/state-store";
import { PENDING_ARRIVAL_FLASH_MS, type PendingArrival, pendingRowMatchesArrival, scrollPendingRowIntoView, subscribePendingArrival } from "./pendingPanelState";
import { collapseComposerStackPanel, composerStackExpandedStore, toggleComposerStackPanel } from "./composerStackAccordion";
import { COMPOSER_STACK_DISCLOSURE_TIMEOUT, composerStackDisclosureSx, swallowStrayDisclosureClick } from "./composerStackDisclosure";

export const DesktopContextShortcut = lazy(async () => {
  const module = await import("./desktop/commands/DesktopContextShortcut");
  return { default: module.DesktopContextShortcut };
});

export const DesktopRowEditToolbar = lazy(async () => {
  const module = await import("./desktop/DesktopRowEditToolbar");
  return { default: module.DesktopRowEditToolbar };
});

export const DesktopPendingEditCommandBindings = lazy(async () => {
  const module = await import("./desktop/commands/DesktopPendingEditShortcuts");
  return { default: module.DesktopPendingEditCommandBindings };
});

export const DesktopRegionShortcut = lazy(async () => {
  const module = await import("./desktop/DesktopRegionShortcut");
  return { default: module.DesktopRegionShortcut };
});

export const DesktopListJumpKeycap = lazy(async () => {
  const module = await import("./desktop/commands/DesktopListJumpKeycap");
  return { default: module.DesktopListJumpKeycap };
});

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
export const TOOLBAR_ICON_BTN = { flexShrink: 0 } as const;

// The mobile fullscreen compose/edit docked BAR — ONE shared bar so Compose and
// Edit look + behave identically (Send, not Cancel/Save). Stacked in the sticky
// docked footer (above the keyboard): attachment thumbnails, then — only when the
// ⚙ toggle is open — the config dropdowns (folded by default so the bar stays a
// single compact row), then the action row: slash / @ / attach / ⚙ on the left,
// Send (submits, or saves the edit) on the right. width:100% + the breakout
// divider make it a full-width docked bar (the shared sheet footer is
// justify-end, which otherwise clusters it).
export function ComposeBar(
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
    onJumpFront,
    submitLabel = "Send",
    submitShortcut = "send",
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
    /** "Jump to front of queue" (no interrupt) — provided only when there's a
     *  queue to jump ahead of. */
    readonly onJumpFront?: (() => void) | undefined;
    /** Semantic label/icon for the primary commit action. Desktop row editors save explicitly; Mobile row editors finish their live-saved edit. */
    readonly submitLabel?: string;
    readonly submitShortcut?: "send" | "save";
    readonly submitIcon?: React.ReactNode;
    /** Desktop-only floating shortcut hints. Mobile keeps the touch toolbar. */
    readonly desktop?: boolean;
  },
): React.JSX.Element {
  // The ⚙ opens the config POPUP (the labeled Mode/Model/Effort dropdowns live in
  // that sheet — see ComposerSheet — not inline on the bar).
  const hasConfig = showSkeleton || options.length > 0;
  // Secondary actions (Save-draft / Jump-to-front) sit inline on a
  // roomy (≥ sm) bar, but fold into a ⋮ overflow on the narrow phone tier — so the
  // PRIMARY buttons keep their full, app-consistent tap target instead of shrinking
  // to cram the whole set onto one row (and Exit-fullscreen never orphans below).
  const theme = useTheme();
  const roomy = useMediaQuery(theme.breakpoints.up("sm"));
  const [moreMenu, setMoreMenu] = useState<HTMLElement | null>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const hasSecondary = Boolean(onSaveDraft || onJumpFront);
  const desktopShortcut = (
    child: ReactNode,
    badge: string,
    enabled = true,
  ): ReactNode =>
    desktop
      ? (
        <Suspense fallback={child}>
          <DesktopContextShortcut badge={badge} enabled={enabled}>
            {child}
          </DesktopContextShortcut>
        </Suspense>
      )
      : child;
  // Leader slots (`␣/`, `␣F`, `␣A`, `␣Z`) for the row editor; the badge
  // lights with the leader while this editor owns focus, and the hover hint
  // names the same path.
  const leaderSlot = (child: ReactNode, key: string): ReactNode =>
    desktopShortcut(child, desktopWorkspaceSequence(key), !dead);
  const leaderHint = (key: string): string | undefined =>
    desktop ? desktopWorkspaceSequence(key) : undefined;
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
      {desktop && (
        <Suspense fallback={null}>
          <DesktopRowEditToolbar
            dead={dead}
            sendable={sendable}
            submitLabel={submitLabel}
            submitShortcut={submitShortcut}
            submitIcon={submitIcon}
            onTrigger={onTrigger}
            onAttach={onAttach}
            onSubmit={onSend}
            onExpand={onExpand}
          />
        </Suspense>
      )}
      {
        /* LEFT-aligned with a fixed gap (not space-evenly — a few icons shouldn't
          stretch across the whole width; reads cleaner next to the attachment
          thumbnail), wrapping to a second row when they don't all fit one line. No
          flex spacer / overflow-scroll / breakout, so nothing clips or pushes off. */
      }
      {!desktop && <Stack
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
        <HintTooltip
          title="Slash command / skill"
          shortcut={leaderHint(DESKTOP_WORKSPACE_KEYS.composerSlash)}
        >
          <span>
            {leaderSlot(
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
              DESKTOP_WORKSPACE_KEYS.composerSlash,
            )}
          </span>
        </HintTooltip>
        <HintTooltip
          title="Reference a file (@)"
          shortcut={leaderHint(DESKTOP_WORKSPACE_KEYS.composerReference)}
        >
          <span>
            {leaderSlot(
              <IconButton
                aria-label="reference a file"
                disabled={dead}
                sx={TOOLBAR_ICON_BTN}
                onClick={(): void => onTrigger("@")}
              >
                <AlternateEmail />
              </IconButton>,
              DESKTOP_WORKSPACE_KEYS.composerReference,
            )}
          </span>
        </HintTooltip>
        {onAttach && (
          <HintTooltip
            title="Attach image or file"
            shortcut={leaderHint(DESKTOP_WORKSPACE_KEYS.composerAttach)}
          >
            <span>
              {leaderSlot(
                <IconButton
                  aria-label="attach image or file"
                  disabled={dead}
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onAttach}
                >
                  <AttachFile />
                </IconButton>,
                DESKTOP_WORKSPACE_KEYS.composerAttach,
              )}
            </span>
          </HintTooltip>
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
        <HintTooltip
          title={submitLabel}
          shortcut={desktop
            ? submitShortcut === "save" ? "Mod+S" : "Mod+Enter"
            : undefined}
        >
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
              submitShortcut === "save" ? `${MOD_LABEL}S` : `${MOD_LABEL}↵`,
              sendable,
            )}
          </span>
        </HintTooltip>
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
          <HintTooltip
            title="Expand editor"
            shortcut={leaderHint(DESKTOP_WORKSPACE_KEYS.editorExpand)}
          >
            <span>
              {leaderSlot(
                <IconButton
                  aria-label="expand editor"
                  sx={TOOLBAR_ICON_BTN}
                  onClick={onExpand}
                >
                  <OpenInFull />
                </IconButton>,
                DESKTOP_WORKSPACE_KEYS.editorExpand,
              )}
            </span>
          </HintTooltip>
        )}
      </Stack>}
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

      </Menu>
    </Stack>
  );
}

// Staged-attachment strip shown above the editor. Image attachments render as
// capped thumbnails; other files as a labelled chip. Each carries a remove
// button. Horizontally scrollable so a handful of attachments never wraps the
// composer or pushes the editor down (the editor must stay reachable on a
// phone). The strip scrolls inside the bar — not flush to the device bottom
// edge — so the iOS bottom-edge gesture doesn't eat the scroll (ui.md §7).
export function AttachmentPreviews({
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
                component={ProtectedImage}
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
            <Close sx={{ fontSize: glyphSize(13) }} />
          </IconButton>
        </Box>
      ))}
    </Stack>
  );
}

// The Zed-style staging panel above the editor — one component for two kinds:
//   - "queued": prompts the busy agent can't take yet, auto-drained one per turn.
//   - "draft":  parked messages the user holds; activated (sent/queued) on demand.

export const uploadPulse = keyframes`
  0%, 100% { opacity: 0.42; transform: translateY(0); }
  50% { opacity: 1; transform: translateY(-1px); }
`;

// An OPTIMISTIC draft/queue row: shown the instant you stage or move it, before
// the service confirms. The local durability barrier and network confirmation
// are both explicit; there is never a visually silent interval after the
// gesture. `failed` is red with retry / return / discard.
export function OptimisticDraftRow({
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
  const saving = appearance === "saving";
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
          : saving
          ? alpha(t.palette.primary.main, 0.05)
          : sending
          ? alpha(t.palette.primary.main, 0.05)
          : syncing
          ? alpha(t.palette.info.main, 0.06)
          : "background.paper",
        ...(failed && { borderLeft: `3px solid ${t.palette.error.main}` }),
      })}
    >
      {(saving || sending) && (
        <CircularProgress
          size={desktopSize(13)}
          thickness={5}
          sx={{ color: "primary.main", mt: 0.25, flexShrink: 0 }}
        />
      )}
      {syncing && (
        <CloudUpload
          aria-hidden
          sx={{
            fontSize: glyphSize(16),
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
            {connected ? "Waiting for Cowboy…" : "Waiting for connection…"}
          </Typography>
        )}
        {(saving || sending) && (
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            {kind === "draft" || saving ? "Saving…" : "Sending…"}
          </Typography>
        )}
        {failed && (
          <Typography variant="caption" sx={{ color: "error.main" }}>
            {message.failure ?? "Couldn't reach Cowboy"}
          </Typography>
        )}
        {previewTray.length > 0 && (
          <QueuedAttachmentChips attachments={previewTray} />
        )}
      </Box>
      {kind === "draft" && !failed && !saving && !sending && (
        <Tooltip title="Send">
          <IconButton
            size="small"
            color="primary"
            aria-label="send draft"
            onClick={(): void => {
              haptic();
              void activateDraft(sessionId, message.id).catch(() => undefined);
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
                  void returnFailedQueued(sessionId, cmid).catch(() => undefined);
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
                void discardQueued(sessionId, cmid).catch(() => undefined);
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

export interface PendingEditController {
  isDirty: () => boolean;
  save: () => Promise<void>;
  discard: () => void;
}

// Collapsible header ("N Queued Messages" / "N Drafts"). Bulk Send all /
// Clear all live in the header kebab so the collapsed bar stays one line.
// Drafts sit BELOW the queue and above the composer (see the Composer render).
export function PendingPanel({
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
  const [queueSettingsOpen, setQueueSettingsOpen] = useState(false);
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
    if (resolution === "save") {
      void controller.save().then(() => {
        setConfirmCollapseEdit(false);
        collapseComposerStackPanel(kind);
      }).catch(() => {
        suppressEditFocusRestoreRef.current = false;
      });
    } else {
      controller.discard();
      setConfirmCollapseEdit(false);
      collapseComposerStackPanel(kind);
    }
  };
  useConfirmEnter(confirmCollapseEdit, () => settleEditAndCollapse("save"));
  const runBulkConfirm = async (): Promise<void> => {
    if (bulkConfirm === "send-all") {
      navigationHaptic();
      await activateAllDrafts(sessionId);
    } else if (bulkConfirm === "clear-all") {
      importantHaptic();
      if (kind === "queued" && items.some((item) => item.currentTurn)) return;
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
  const toggleTap = useReliableTouchTap<HTMLButtonElement>(
    toggleCollapsed,
    swallowStrayDisclosureClick,
  );
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
  useEffect(() => {
    if (editingId !== null || items.length === 0) return;
    const recoveredId = recoverPendingEditId(sessionId, kind, items);
    if (recoveredId !== null) {
      setEditingId(recoveredId);
    }
  }, [editingId, items, kind, sessionId]);
  // Reorder is a low-frequency action, so the per-row drag grips are hidden by
  // default (they'd waste ~40px on every row of a narrow phone) and revealed
  // only in this opt-in "reorder mode" (iOS list-Edit pattern). Local + ephemeral
  // like `collapsed`. Per-panel state → drafts and queue toggle independently.
  const [reordering, setReordering] = useState(false);
  // Disclosure, arrival flash and reorder mode re-render this panel but never
  // change a row. Every fold used to re-render every PendingRow, the heaviest
  // subtree here; the first fold after a load runs that code cold and cost
  // about twice the later ones. Rows are memoized (PendingPanelRow), so their
  // callbacks must keep one identity; the parent's inline closures are read
  // through this ref instead.
  const rowCallbacksRef = useRef({
    commands,
    onMoveDraft,
    onScheduleDraft,
    onResumeKeyboardSurface,
  });
  rowCallbacksRef.current = {
    commands,
    onMoveDraft,
    onScheduleDraft,
    onResumeKeyboardSurface,
  };
  const rowCommands = useCallback(
    (): AvailableCommand[] => rowCallbacksRef.current.commands(),
    [],
  );
  const resumeRowKeyboardSurface = useCallback((): void => {
    rowCallbacksRef.current.onResumeKeyboardSurface?.();
  }, []);
  const moveRow = useCallback((id: string): void => {
    rowCallbacksRef.current.onMoveDraft?.(id);
  }, []);
  const scheduleRow = useCallback((id: string): void => {
    rowCallbacksRef.current.onScheduleDraft?.(id);
  }, []);
  const editRow = useCallback((id: string): void => {
    setReordering(false);
    setEditingId(id);
  }, []);
  const finishRowEdit = useCallback((id: string): void => {
    const restoreFocus = !suppressEditFocusRestoreRef.current;
    suppressEditFocusRestoreRef.current = false;
    setEditingId(null);
    if (restoreFocus) {
      requestAnimationFrame(() =>
        scrollRef.current?.querySelector<HTMLElement>(
          `[data-desktop-item="${CSS.escape(id)}"]`,
        )?.focus({ preventScroll: true })
      );
    }
  }, []);
  const count = items.length;
  const nativeItems = items.filter((item) => item.currentTurn);
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
    ids: items.filter((m) => !m.currentTurn).map((m) => m.id),
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
        mb: desktop ? 0.5 : 0,
        // Only Desktop adopts the borderless disclosure rail. Mobile keeps its
        // established staging cards and touch geometry.
        ...(desktop ? {
          border: 0,
          borderBottom: 1,
          borderColor: "divider",
          borderRadius: 0,
          overflow: "hidden",
          bgcolor: "transparent",
          "&:focus-within": { borderColor: "primary.main" },
        } : {
          ...mobileComposerPanelFrameSx,
          bgcolor: kind === "draft" ? "action.selected" : "action.hover",
        }),
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
        // Desktop is a compact utility rail; Mobile keeps native touch height.
        sx={{
          display: mobileFloatingEdit ? "none" : "flex",
          pr: desktop ? 0.5 : 0.75,
          py: 0,
          minHeight: desktop ? desktopSize(32) : mobileComposerPanelHeaderMinHeight,
          ...(desktop && {
            "& > .MuiIconButton-root": {
              width: desktopSize(32),
              height: desktopSize(32),
            },
          }),
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
            ...(desktop && { "&:focus-visible": { bgcolor: "action.hover" } }),
          }}
        >
          {/* Desktop: the Calls/Plan chevron column (22 + 0.375rem gap), so
              all three chevrons share a centre, not only the labels. */}
          <Box
            sx={{
              width: desktop ? desktopSize(22) : 40,
              mr: desktop ? "0.375rem" : 0,
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
            {desktop
              ? kind === "draft" ? "Drafts" : "Queue"
              : `${count} ${noun}${count === 1 ? "" : "s"}`}
          </Typography>
          {desktop && (
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ ml: 0.75, fontVariantNumeric: "tabular-nums", flexShrink: 0 }}
            >
              {count}
            </Typography>
          )}
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
            <Box
              sx={{
                display: "none",
                "@container pendingPanel (min-width: 30rem)": {
                  display: "contents",
                },
              }}
            >
              <Suspense
                fallback={
                  <ShortcutKeycap
                    keyLabel="'"
                    variant="context"
                    availability="inactive"
                    sx={{ ml: 0.75 }}
                  />
                }
              >
                <DesktopListJumpKeycap
                  region={`prompt.${kind}`}
                  keyLabel="'"
                  prefix
                  sx={{ ml: 0.75 }}
                />
              </Suspense>
            </Box>
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
            <Stack
              direction="row"
              spacing={0.5}
              alignItems="center"
              sx={{
                display: "none",
                "@container pendingPanel (min-width: 24rem)": { display: "flex" },
              }}
            >
              {kind === "queued" && (
                <Suspense fallback={null}>
                  <DesktopRegionShortcut
                    shortcut={DESKTOP_SHORTCUTS.focusQueue}
                    title={visuallyCollapsed
                      ? "Open and focus queue"
                      : "Close queue when focused"}
                    singleKeycap={DESKTOP_SHORTCUTS.focusQueue}
                  />
                </Suspense>
              )}
              {kind === "draft" && (
                <Suspense fallback={null}>
                  <DesktopRegionShortcut
                    shortcut={DESKTOP_SHORTCUTS.focusDrafts}
                    title="Focus drafts"
                    singleKeycap={DESKTOP_SHORTCUTS.focusDrafts}
                  />
                </Suspense>
              )}
            </Stack>
          )}
          {desktop && reordering && (
            <Typography
              variant="caption"
              color="primary.main"
              sx={{
                ml: 0.75,
                minWidth: 0,
                fontWeight: 700,
                letterSpacing: "0.04em",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
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
        {count >= 2 && (!desktop || !visuallyCollapsed) && (
          <HintTooltip title={reordering ? "Done" : "Reorder"}>
            {/* A disabled button emits no pointer events; the span keeps the hint. */}
            <Box
              component="span"
              sx={{
                display: "inline-flex",
                flexShrink: 0,
                [ROW_ACTIONS_INLINE]: { display: "none" },
              }}
            >
              <IconButton
                size="small"
                disabled={editingId !== null}
                aria-label={reordering ? "done reordering" : "reorder"}
                color={reordering ? "primary" : "default"}
                onClick={(): void =>
                  setReordering((r) => {
                    if (!r) haptic(); // light — entering reorder mode (grips now live)
                    return !r;
                  })}
              >
                <SwapVert fontSize="small" />
              </IconButton>
            </Box>
          </HintTooltip>
        )}
        <IconButton
          size="small"
          disabled={editingId !== null}
          aria-label={kind === "draft" ? "Draft actions" : "Queue actions"}
          aria-haspopup={desktop ? "menu" : undefined}
          aria-expanded={desktop ? panelMenuEl !== null : undefined}
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
          {kind === "queued" && (
            <MenuItem
              onClick={(): void => {
                setPanelMenuEl(null);
                setQueueSettingsOpen(true);
              }}
            >
              <ListItemIcon>
                <Tune fontSize="small" />
              </ListItemIcon>
              <ListItemText>Queue delivery</ListItemText>
            </MenuItem>
          )}
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
            disabled={editingId !== null || nativeItems.length > 0}
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
        timeout={COMPOSER_STACK_DISCLOSURE_TIMEOUT}
        onEntered={revealArrivalRow}
        sx={{
          ...composerStackDisclosureSx(!visuallyCollapsed),
          // Layout containment can leave iOS WebKit's parent overflow layer
          // with a stale scroll range until a later layout pass. That
          // presents as a temporarily frozen Queue/Drafts stack after a
          // disclosure, so only Desktop takes the paint isolation.
          ...(desktop && {
            "& .MuiCollapse-wrapperInner": { contain: "layout paint" },
          }),
        }}
      >
        <Stack
          spacing={desktop ? 0 : 0.5}
          ref={scrollRef}
          data-mobile-pending-scrollport={!desktop && !visuallyCollapsed
            ? "true"
            : undefined}
          data-desktop-pending-list={desktop ? "true" : undefined}
          data-desktop-aux-list={desktop ? "true" : undefined}
          sx={{
            // Keep the row arrival outline clear of the inner scrollport edge.
            px: mobileFloatingEdit ? 0 : 0.5,
            pt: mobileFloatingEdit ? 0 : 0.5,
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
          {nativeItems.length > 0 && <Box sx={{ px: 1, pb: 1 }}>
            <Typography variant="caption" color="text.secondary">Current turn</Typography>
            {nativeItems.map((item) => <NativeDeliveryRow key={item.id} sessionId={sessionId} row={item} />)}
            {sortable.order.length > 0 && <Typography variant="caption" color="text.secondary">Next turn</Typography>}
          </Box>}
          {sortable.order.map((id) => {
            const index = sortable.order.indexOf(id);
            const m = byId.get(id);
            if (!m) return null;
            // A LOCAL optimistic draft (carries `status`) renders a lightweight
            // row with no grip / edit / reorder — it isn't a server item yet.
            const optimistic = m.status !== undefined;
            const leadingHandle = editingId !== m.id && !optimistic &&
              count > 1;
            const gripSize = 44;
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
                sx={desktop ? desktopPendingRowSx(index, leadingHandle) : undefined}
              >
                {
                  /* The leading slot is the reorder grip. Wide rows show it; narrow
                    rows and Mobile allocate nothing until reorder mode. Jumping
                    uses transient `'` labels (FOCUS.md "Labels"), never a
                    per-row number. A single row needs no ordering affordance. */
                }
                {leadingHandle && desktop && (
                  // Desktop is keyboard-first: `O` pins reorder mode (J/K
                  // move, Esc done) and Shift+J/K moves directly. The pointer
                  // grip is a slim edge handle revealed on hover or focus, not
                  // a permanent gutter beside every message.
                  <Box
                    className="cowboy-pending-grip"
                    sx={{
                      position: "absolute",
                      left: 0,
                      top: 0,
                      bottom: 0,
                      // rem: the Desktop grip follows the global font size.
                      width: "1.25rem",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      opacity: reordering ? 1 : 0,
                      transition: "opacity 120ms ease",
                    }}
                  >
                    <Suspense fallback={null}>
                      <DesktopContextShortcut
                        badge="O"
                        itemScoped
                        placement="corner"
                      >
                        <HintTooltip
                          title="Drag to reorder"
                          shortcut="O"
                          detail="J/K move · Shift+J/K moves directly · Esc done"
                        >
                          <IconButton
                            {...sortable.handleProps(m.id)}
                            aria-label="Drag to reorder"
                            sx={{
                              width: "1.25rem",
                              height: "2rem",
                              p: 0,
                              borderRadius: 1,
                              color: "text.disabled",
                              cursor: "grab",
                              "& .MuiSvgIcon-root": { fontSize: "1rem" },
                            }}
                          >
                            <DragIndicator />
                          </IconButton>
                        </HintTooltip>
                      </DesktopContextShortcut>
                    </Suspense>
                  </Box>
                )}
                {leadingHandle && !desktop && (
                  <Box
                    sx={{
                      position: "relative",
                      width: gripSize,
                      height: gripSize,
                      flexShrink: 0,
                      display: reordering ? "inline-flex" : "none",
                      [ROW_ACTIONS_INLINE]: {
                        display: "inline-flex",
                        width: gripSize,
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
                        width: gripSize,
                        height: gripSize,
                        color: "text.disabled",
                        display: reordering ? "inline-flex" : "none",
                        [ROW_ACTIONS_INLINE]: { display: "inline-flex" },
                      }}
                    >
                      <DragIndicator fontSize="small" />
                    </IconButton>
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
                      <PendingPanelRow
                        desktop={desktop}
                        kind={kind}
                        sessionId={sessionId}
                        message={m}
                        status={status}
                        commands={rowCommands}
                        editing={editingId === m.id}
                        keyboardOpen={keyboardOpen}
                        onResumeKeyboardSurface={onResumeKeyboardSurface
                          ? resumeRowKeyboardSurface
                          : undefined}
                        onEditRow={editRow}
                        onEditRowDone={finishRowEdit}
                        onEditController={registerEditController}
                        onMoveRow={onMoveDraft ? moveRow : undefined}
                        onScheduleRow={onScheduleDraft ? scheduleRow : undefined}
                      />
                    )}
                </Box>
              </Stack>
            );
          })}
        </Stack>
      </Collapse>
      <ConfirmSheet
        open={queueSettingsOpen}
        onClose={(): void => setQueueSettingsOpen(false)}
        title="Queue delivery"
      >
        <QueueDeliverySettings sessionId={sessionId} />
      </ConfirmSheet>
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
// Edit / Delete. Desktop edit mode is transactional: Mod+S saves and Esc
// asks before discarding. The primary action depends on whether the session can take a turn
// right now: dispatchable → a plain "Send now" (sends immediately, revives a
// dead session); busy messages remain queued until the active turn ends.

// Container-query breakpoint for a row's secondary actions (see PendingPanel's
// containerName). Above this panel width the row shows every action inline; below
// it they fold into the kebab. ~520px: comfortably fits Send + 3 icons + readable
// text on an iPad/desktop panel, while every portrait phone panel (≤ ~410px) stays
// kebab. A landscape phone (panel ~900px) genuinely has the room, so inline there
// is correct too — exactly what keying on real width (not device class) buys.
export const ROW_ACTIONS_INLINE = "@container pendingPanel (min-width: 520px)";

/** Desktop Queue/Draft rows read as lines of the Prompt, not cards: flat,
 *  hairline-separated, with the current row marked by a primary edge (an
 *  editor's current line) instead of an outlined capsule. */
export function desktopPendingRowSx(index: number, reorderable: boolean) {
  const current = {
    bgcolor: (t: Theme) => alpha(t.palette.primary.main, 0.05),
    boxShadow: (t: Theme) => `inset 2px 0 0 ${t.palette.primary.main}`,
  };
  return {
    position: "relative",
    pl: reorderable ? "20px" : 0,
    borderTop: index > 0 ? 1 : 0,
    borderColor: "divider",
    outline: "none",
    transition: "background-color 120ms ease, box-shadow 120ms ease",
    "&:hover": {
      bgcolor: (t: Theme) => alpha(t.palette.primary.main, 0.025),
    },
    "&:focus, &:focus-within, &[data-desktop-current='true']": current,
    "&:hover .cowboy-pending-grip, &:focus .cowboy-pending-grip, &:focus-within .cowboy-pending-grip":
      { opacity: 1 },
  } as const;
}

export function pendingContentCleared(
  text: string,
  attachments: Attachment[],
): boolean {
  return stripImageTokens(text).trim() === "" && attachments.length === 0;
}

export type PendingRowProps = Parameters<typeof PendingRow>[0];

// A row only re-renders when its own inputs change. The panel passes id-taking
// callbacks with a stable identity; the per-row closures are built here.
export const PendingPanelRow = memo(function PendingPanelRow({
  onEditRow,
  onEditRowDone,
  onMoveRow,
  onScheduleRow,
  ...row
}: Omit<PendingRowProps, "onEdit" | "onEditDone" | "onMove" | "onSchedule"> & {
  onEditRow: (id: string) => void;
  onEditRowDone: (id: string) => void;
  onMoveRow?: ((id: string) => void) | undefined;
  onScheduleRow?: ((id: string) => void) | undefined;
}): React.JSX.Element {
  const id = row.message.id;
  return (
    <PendingRow
      {...row}
      onEdit={(): void => onEditRow(id)}
      onEditDone={(): void => onEditRowDone(id)}
      onMove={onMoveRow ? (): void => onMoveRow(id) : undefined}
      onSchedule={onScheduleRow ? (): void => onScheduleRow(id) : undefined}
    />
  );
});

export function PendingRow({
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
  const recoveredEdit = useRef(
    getPendingEdit(sessionId, kind, message.id),
  ).current;
  const initialEditText = useRef(
    recoveredEdit?.text ??
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
  const callNotice = pendingCallNotice(message);
  // Per-row kebab (⋮) position. Click coordinates, not the button node: the
  // peek/footer keep a translate3d layer, and hiding the kebab via container
  // query can detach the element so MUI's anchorEl lands at (0, 0).
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(
    null,
  );
  const suppressEditTapUntilRef = useRef(0);
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
    const initial = recoveredEdit?.attachments ?? message.attachments;
    seedInlineAttachments(initial);
    return initial;
  });
  const editAttachmentsPending = editAttachments.some((attachment) =>
    attachment.pending
  );
  const updateEditDraft = (next: string): void => {
    const previous = editTextRef.current;
    editTextRef.current = next;
    setEditAttachments((current) =>
      reconcileDeletedInlineImages(previous, next, current, getInlineAttachment)
    );
    setDraft(next);
  };
  const editorRef = useRef<ComposerEditorHandle>(null);
  const rowRef = useRef<HTMLDivElement>(null);
  // Per-row delete confirm. Dropping a queued message / draft is irreversible, so
  // the × opens this modal instead of deleting on a single tap.
  const [confirmRemove, setConfirmRemove] = useState(false);
  // A queued message while the session works: the same delivery choices as
  // the Prompt's busy Send, scoped to a message that is already queued.
  const [deliveryAnchor, setDeliveryAnchor] = useState<HTMLElement | null>(null);
  const queuedAhead = useStoreSelector((snapshot) =>
    kind === "queued"
      ? Math.max(
        0,
        (snapshot.queues.get(sessionId) ?? []).findIndex((row) => row.id === message.id),
      )
      : 0
  );
  const queuePaused = useStoreSelector((snapshot) =>
    snapshot.sessions.find((session) => session.id === sessionId)?.paused ?? false
  );
  const queueSteerCapable = useStoreSelector((snapshot) =>
    snapshot.sessions.find((session) => session.id === sessionId)
      ?.supports_current_turn === true
  );
  const queueConnected = useConnected();
  const steerQueued = (
    target: import("./currentTurnDelivery").DeliveryTarget | undefined,
    nextCheckpoint = false,
  ): void => {
    setDeliveryAnchor(null);
    // The current-turn row persists the content first and owns its receipt
    // (a failed delivery is retained there), so removing the queued original
    // afterwards can neither lose the message nor send it twice.
    void currentTurnPrompt(
      sessionId,
      message.text,
      message.attachments,
      target,
      nextCheckpoint,
    )
      .then(() => removeQueued(sessionId, message.id))
      .catch((error: Error) => notify(error.message));
  };
  const [confirmDiscardEdit, setConfirmDiscardEdit] = useState(false);
  const doRemove = (): void => {
    importantHaptic();
    const removal = kind === "draft"
      ? removeDraft(sessionId, message.id)
      : removeQueued(sessionId, message.id);
    void removal.catch(() => undefined);
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
    const recovery = getPendingEdit(sessionId, kind, message.id);
    const nextText = recovery?.text ?? seedText;
    const nextAttachments = recovery?.attachments ?? seedAttachments;
    setPendingEdit({
      sessionId,
      kind,
      id: message.id,
      text: nextText,
      attachments: nextAttachments,
      baseText: message.text,
      baseAttachments: message.attachments,
    });
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
        setDraft(nextText);
        editTextRef.current = nextText;
        setEditAttachments(nextAttachments);
        onEdit();
      });
      editorRef.current?.focusEnd();
      return;
    }
    setDraft(nextText);
    editTextRef.current = nextText;
    setEditAttachments(nextAttachments);
    onEdit();
  };
  const pendingEditTap = useReliableTouchTap<HTMLDivElement>(() => {
    beginEdit();
  });
  const discardEdit = (): void => {
    clearPendingEdit(sessionId, kind, message.id);
    setConfirmDiscardEdit(false);
    setDraft(seedText);
    editTextRef.current = seedText;
    setEditAttachments(seedAttachments);
    setOverlayOpen(false);
    onEditDone();
  };
  // A recovered edit can open before its attachment bytes are back from the
  // database (a reload reopens the editor at once). Adopt them when they
  // arrive, and drop the tokens of images that turned out to be gone.
  useEffect(() => {
    if (!editing) return undefined;
    return subscribePendingEditRestore(sessionId, kind, message.id, (restored) => {
      seedInlineAttachments(restored.attachments);
      const restoredIds = new Set(restored.attachments.map((attachment) => attachment.id));
      for (const token of imageTokensInText(editTextRef.current)) {
        if (restoredIds.has(token.id) || getInlineAttachment(token.id) !== undefined) continue;
        overlayEditorRef.current?.deleteImage(token.id);
        editorRef.current?.deleteImage(token.id);
      }
      setEditAttachments((current) => {
        const held = new Set(current.map((attachment) => attachment.id));
        const missing = restored.attachments.filter((attachment) => !held.has(attachment.id));
        return missing.length === 0 ? current : [...current, ...missing];
      });
      overlayEditorRef.current?.refreshImages();
      editorRef.current?.refreshImages();
    });
  }, [editing, kind, message.id, sessionId]);
  const liveEditText = (): string =>
    pendingEditLiveText(
      overlayEditorRef.current?.getValue() ?? editorRef.current?.getValue(),
      editTextRef.current,
    );
  const persistEdit = async (): Promise<boolean> => {
    // A synchronous paste placeholder has no ACP bytes yet. Keep the editor
    // alive until encoding either promotes or removes it; persisting the empty
    // block would make the pending row restore a broken inline token.
    if (editAttachmentsPending) return false;
    const text = liveEditText();
    editTextRef.current = text;
    setDraft(text);
    if (pendingContentCleared(text, editAttachments)) {
      if (kind === "draft") await removeDraft(sessionId, message.id);
      else await removeQueued(sessionId, message.id);
      clearPendingEdit(sessionId, kind, message.id);
      setOverlayOpen(false);
      onEditDone();
      return false;
    }
    if (kind === "draft") {
      await editDraft(sessionId, message.id, text, editAttachments);
    } else await editQueued(sessionId, message.id, text, editAttachments);
    clearPendingEdit(sessionId, kind, message.id);
    setCommittedText(text);
    setCommittedAttachments(editAttachments);
    return true;
  };
  useEffect(() => {
    if (!editing) return;
    setPendingEdit({
      sessionId,
      kind,
      id: message.id,
      text: draft,
      attachments: editAttachments,
      baseText: message.text,
      baseAttachments: message.attachments,
    });
  }, [
    draft,
    editAttachments,
    editing,
    kind,
    message.attachments,
    message.id,
    message.text,
    sessionId,
  ]);
  const saveEdit = async (): Promise<void> => {
    if (!await persistEdit()) return;
    setOverlayOpen(false);
    onEditDone();
  };
  const requestSaveEdit = (): void => {
    void saveEdit().catch(() => undefined);
  };
  const completePendingDelivery = async (
    operation: () => Promise<void>,
  ): Promise<void> => {
    if (!await persistEdit()) return;
    await operation();
    setOverlayOpen(false);
    onEditDone();
  };
  const sendDraftFromEdit = (): Promise<void> =>
    kind === "draft"
      ? completePendingDelivery(() => activateDraft(sessionId, message.id))
      : Promise.resolve();
  const scheduleDraftFromEdit = async (): Promise<void> => {
    if (kind !== "draft" || onSchedule === undefined) return;
    // The schedule sheet targets the durable draft id. Persist the current
    // inline buffer before opening it so scheduling never sends stale text.
    if (!await persistEdit()) return;
    setOverlayOpen(false);
    onEditDone();
    onSchedule();
  };
  // Mobile Queue/Draft edits own the writing surface while the keyboard is
  // up. Dismissing it must persist the live buffer, then return to the
  // compact card — never leave the two-track dock over an empty canvas.
  const mobileEditSawKeyboardRef = useRef(false);
  const persistEditRef = useRef(persistEdit);
  persistEditRef.current = persistEdit;
  const finishMobileEdit = (): void => {
    if (!touchInput || editAttachmentsPending) return;
    void persistEditRef.current().then(() => {
      setOverlayOpen(false);
      onEditDone();
      dismissMobileSoftwareKeyboard();
      releaseMobileComposerFocus();
    }).catch(() => undefined);
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
        () => {
          void persistEditRef.current().catch(() => undefined);
        },
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
      save: () => Promise<void>;
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
      save: () => latestEditTransactionRef.current?.save() ?? Promise.resolve(),
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
    // Mod+S saves and Esc asks before throwing away the local buffer.
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
        onSend={requestSaveEdit}
        submitLabel="Save changes"
        submitShortcut="save"
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
            ...(desktop
              ? { p: 0.75, border: 0, borderRadius: 0, bgcolor: "transparent" }
              : {
              borderRadius: mobileComposerPanelFrameSx.borderRadius,
              bgcolor: "transparent",
              borderColor: mobileComposerOutlineColor,
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
                onDone={requestSaveEdit}
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
                onSubmit={desktop ? undefined : requestSaveEdit}
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
                          void scheduleDraftFromEdit().catch(() => undefined);
                        }}
                      >
                        <Schedule />
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
              if (touchInput) {
                void persistEdit().then((saved) => {
                  if (saved) setOverlayOpen(false);
                }).catch(() => undefined);
              } else void saveEdit().catch(() => undefined);
            }}
            onSaveDraft={(): void => {
              if (touchInput) {
                void persistEdit().then((saved) => {
                  if (saved) setOverlayOpen(false);
                }).catch(() => undefined);
              } else void saveEdit().catch(() => undefined);
            }}
            onCollapse={(): void => {
              void persistEdit().then((saved) => {
                if (saved) setOverlayOpen(false);
              }).catch(() => undefined);
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
            submitLabel={touchInput ? "Collapse editor" : "Save changes"}
            saveOnly={desktop}
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
      ...(!message.schedule ? [{
        key: "document",
        label: "Move to independent Drafts",
        icon: <DescriptionOutlined fontSize="small" />,
        onClick: (): void => { void moveSessionDraftToDocument(sessionId, message).catch((error: Error) => notify(error.message)); },
      }] : []),
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
        onClick: (): void => {
          void queuedToDraft(sessionId, message.id).catch(() => undefined);
        },
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
  // Row keys while the item owns focus (`L` also answers to Enter).
  const secondaryShortcut: Record<string, { badge: string }> = {
    edit: { badge: "L" },
    schedule: { badge: "T" },
    move: { badge: "M" },
    document: { badge: "D" },
    return: { badge: "R" },
    remove: { badge: "X" },
  };

  // Primary action — always inline. Drafts always Send (send-or-queue); a queued
  // row sends now only when the session is free.
  // Built as a statement to avoid a nested ternary in the JSX.
  let primary: React.JSX.Element;
  const primaryEnabled = true;
  if (kind === "draft") {
    primary = (
      <Tooltip title={dispatchable ? "Send" : "Add to queue"}>
        <NetworkIconButton
          data-desktop-item-action="default"
          size="small"
          color="primary"
          aria-label="send draft"
          reliableTouch={!desktop}
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
          reliableTouch={!desktop}
          networkAction={() => requestSendQueued(sessionId, message.id)}
        >
          <Send fontSize="small" />
        </NetworkIconButton>
      </Tooltip>
    );
  } else {
    // Busy: never a dead button. Steer it into the current turn, or let it
    // run first when this turn ends.
    primary = (
      <Tooltip title="Send: Steer into the current turn, or run it first">
        <IconButton
          data-desktop-item-action="default"
          size="small"
          color="primary"
          aria-label="send now"
          aria-haspopup="menu"
          aria-expanded={deliveryAnchor !== null}
          onClick={(event): void => setDeliveryAnchor(event.currentTarget)}
        >
          <Send fontSize="small" />
        </IconButton>
      </Tooltip>
    );
  }

  if (desktop) {
    const primaryControl = primary;
    primary = (
      <Suspense fallback={primaryControl}>
        <DesktopContextShortcut
          badge="S"
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
        ...(desktop && {
          border: 0,
          borderRadius: 0,
          bgcolor: "transparent",
        }),
        display: "flex",
        alignItems: callNotice ? "center" : "flex-start",
        minHeight: callNotice ? 0 : mobilePendingRowMinHeight,
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
          alignSelf: callNotice ? "center" : "stretch",
          minWidth: 0,
          minHeight: callNotice ? 0 : mobileComposerIdleEditorMinHeight,
          cursor: touchInput ? "pointer" : "text",
        }}
      >
        {callNotice
          ? <CallNoticeLine notice={callNotice} sessionId={sessionId} />
          : seedText.trim() !== "" && (
            <MessagePreview text={seedText} attachments={seedAttachments} />
          )}
        {seedTray.length > 0 && (
          <QueuedAttachmentChips attachments={seedTray} />
        )}
        {/* Scheduled-draft badge: info chip for when it fires; tap to reschedule. Info, never red: a schedule is a notice, not a failure (conventions/ui.md §4). */}
        {message.schedule && (
          <Chip
            data-pending-content-action="schedule"
            size="small"
            clickable
            icon={<Schedule sx={{ fontSize: glyphSize(15) }} />}
            label={`${fireLabel(message.schedule.fire_at_ms)} · ${
              message.schedule.delivery === "front" ? "队首" : "队尾"
            }`}
            onClick={onSchedule}
            sx={{
              mt: 0.75,
              height: 24,
              borderRadius: 999,
              fontSize: desktopSize(12),
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
        sx={{
          flexShrink: 0,
          // Desktop: the row's actions follow the global font size. The
          // shared IconButton floor (32px) held them still below ~73% (the
          // default is 65%), so only the glyph shrank inside a fixed box. A
          // 24px floor is the pointer-target minimum (WCAG 2.2 AA).
          ...(desktop && {
            "& .MuiIconButton-root": {
              width: `max(24px, ${desktopSize(36)})`,
              height: `max(24px, ${desktopSize(36)})`,
            },
          }),
        }}
      >
        {primary}
        {kind === "queued" && deliveryAnchor && (
          <DeliveryPicker
            sessionId={sessionId}
            anchor={deliveryAnchor}
            busy={status === "busy"}
            steerReady={queueConnected && status === "busy" && queueSteerCapable}
            paused={queuePaused}
            queueCount={queuedAhead}
            queued
            onClose={(): void => setDeliveryAnchor(null)}
            onNextTurn={(): void => setDeliveryAnchor(null)}
            onQueueFront={(): void => {
              setDeliveryAnchor(null);
              void requestSendQueued(sessionId, message.id)
                .catch((error: Error) => notify(error.message));
            }}
            onSteer={steerQueued}
            onSendAndPause={(): void => setDeliveryAnchor(null)}
            onStop={(): void => {
              setDeliveryAnchor(null);
              cancelTurn(sessionId);
            }}
          />
        )}
        {/* Secondary actions — inline on a roomy (iPad/desktop) panel … */}
        <Stack
          direction="row"
          alignItems="center"
          sx={{ display: "none", [ROW_ACTIONS_INLINE]: { display: "flex" } }}
        >
          {secondary.map((a) => (
            <HintTooltip
              key={a.key}
              title={a.label}
              shortcut={desktop ? secondaryShortcut[a.key]?.badge : undefined}
            >
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
            </HintTooltip>
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
