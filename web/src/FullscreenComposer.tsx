import {
  type ReactNode,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  Box,
  Button,
  IconButton,
  Tooltip,
  useTheme,
} from "@mui/material";
import {
  AttachFile,
  Bolt,
  Check,
  Close,
  CloseFullscreen,
  EditNoteOutlined,
  KeyboardHide,
  Schedule,
  Send,
} from "@mui/icons-material";
import {
  type ComposerEditorHandle,
  type ComposerEditorSelection,
  PlatformComposerEditor,
} from "./composer/PlatformComposerEditor";
import { useComposerToolbar } from "./composerToolbarConfig";
import { ComposerToolbarSettings } from "./ComposerToolbarSettings";
import { MobileComposerFormatActions } from "./MobileComposerFormatActions";
import {
  MobileComposerAccessoryButton,
  MobileComposerAccessoryDock,
} from "./MobileComposerAccessoryDock";
import { mobileComposerKeyboardGap } from "./mobileComposerPrimitives";
import { useKeyboardOpen } from "./keyboardInset";
import {
  isMobileEditorFocusTransferPending,
  releaseMobileComposerFocus,
} from "./composer/mobileComposerFocus";
import type { NativeClipboardImagePasteRequest } from "./composer/nativeClipboardImagePaste";
import { isNativeShell } from "./nativeShell";
import type { AvailableCommand } from "./protocol";
import { haptic } from "./haptic";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { ConfirmSheet } from "./Sheet";

// Brand-new full-screen mobile compose surface (NOT a DetentSheet): a fixed
// 100dvh overlay modeled on Obsidian's mobile note editor — a full-height native
// text canvas on touch (promoted to CM6 only for inline-image widgets) and a
// SELECTION-AWARE markdown toolbar pinned just above the keyboard. The editor is
// the same engine as every other surface, so markdown stays the literal value and
// nothing re-serializes on open/close. The native iOS keyboard accessory bar is
// suppressed by cowboy's Tauri keyboard-bar swizzle; this toolbar replaces it.
export function FullscreenComposer({
  value,
  onChange,
  onSubmit,
  onSaveDraft,
  onCollapse,
  onAttach,
  onPasteFiles,
  onPasteClipboardImages,
  sessionId,
  commands,
  placeholder,
  sendable,
  attachmentsSlot,
  editorRef,
  autoFocus = true,
  showCollapse = true,
  submitLabel = "Send",
  submitIcon,
  vim = false,
  onVimMode,
  onDiscard,
  onSchedule,
  onForcePush,
  forcePushEnabled = false,
}: {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onSaveDraft: () => void;
  onCollapse: () => void;
  onAttach: () => void;
  onPasteFiles: (
    files: File[],
    selection?: ComposerEditorSelection,
  ) => void;
  onPasteClipboardImages: (
    request: NativeClipboardImagePasteRequest,
  ) => Promise<void> | void;
  sessionId: string;
  commands: () => AvailableCommand[];
  placeholder: string;
  sendable: boolean;
  attachmentsSlot?: ReactNode;
  // The SHARED composer editor handle. Must be the same ref the parent's
  // `addFiles`/submit/clear use — else paste-an-image here inserts the inline
  // token into nobody (the parent's inline editor is unmounted while fullscreen is
  // open), so the image attaches + sends but shows NO inline thumbnail.
  editorRef: RefObject<ComposerEditorHandle | null>;
  // Whether to programmatically focus the editor on open (timers below). The main
  // composer's expand focuses IN the user tap (flushSync) so it can leave this
  // FALSE — a programmatic focus there would re-focus over the armed one and could
  // disarm the iOS long-press menu. The row-edit overlay (opened from an effect,
  // not directly in a tap) still relies on this.
  autoFocus?: boolean;
  /** Keep collapse separate from submit for new messages. Row edits hide it:
   *  their live-saved Done action already closes the overlay. */
  showCollapse?: boolean;
  /** Row editors are live-saved; their primary action finishes editing rather
   *  than sending the queued/draft item. */
  submitLabel?: string;
  submitIcon?: ReactNode;
  /** Desktop preference. PlatformComposerEditor always forces this off on
   * touch surfaces, so the native Mobile editor path remains unchanged. */
  vim?: boolean;
  onVimMode?: (mode: string) => void;
  /** Edit mode: save explicitly in the footer; abandoning edits requires confirmation. */
  onDiscard?: (() => void) | undefined;
  /** Main-composer delivery actions. Row edits intentionally omit them. */
  onSchedule?: (() => void) | undefined;
  onForcePush?: ((anchor: HTMLElement) => void) | undefined;
  forcePushEnabled?: boolean;
}): React.JSX.Element {
  const nativeShell = isNativeShell();
  const keyboardOpen = useKeyboardOpen();
  const theme = useTheme();
  const sawKeyboardRef = useRef(false);
  useEffect(() => {
    if (keyboardOpen) {
      sawKeyboardRef.current = true;
      return;
    }
    if (!showCollapse || !sawKeyboardRef.current) return;
    if (isMobileEditorFocusTransferPending()) return;
    onCollapse();
  }, [keyboardOpen, onCollapse, showCollapse]);

  // CM6 is UNCONTROLLED, exactly like the inline composer: freeze the open-time
  // text as a one-shot seed and let `onChange` flow text OUT only. Passing the live
  // `value={text}` (what we used to do) makes @uiw/react-codemirror reconcile the
  // doc on EVERY keystroke — and worse, on every IME composition update — which
  // bounces the iOS caret and corrupts mid-composition pinyin ("状态错乱"). The
  // inline editor's own comment warns against `value={text}` for exactly this; the
  // fullscreen surface must follow the same rule. This component remounts on each
  // open (parent gates it behind `composeFs`), so the frozen seed is always the
  // current in-progress text at open time; on close the parent syncs it back.
  const seed = useRef(value).current;

  // Fallback for a host that does not transfer focus while mounting. Cowboy's
  // main and row-edit entry points pass false and own one user-gesture/layout
  // transfer; repeated timers disarm the native iOS text menu.
  useEffect(() => {
    if (!autoFocus) return undefined;
    const timer = globalThis.setTimeout(() => editorRef.current?.focusEnd(), 0);
    return (): void => globalThis.clearTimeout(timer);
  }, [autoFocus]);

  const act = (fn: () => void): () => void => () => {
    haptic();
    fn();
  };

  // CONFIG-DRIVEN Obsidian-style toolbar: render the user's ordered command ids
  // (composerToolbarConfig) from the shared registry (composerCommands), instead
  // of a hardcoded array. Each command runs against { editor, attach } — `attach`
  // drives the host file-picker (attachment isn't an editor-only action), which
  // is why a command takes a context. Actions work with OR without a selection;
  // each re-focuses the editor (keyboard stays up). Scrolls horizontally on a
  // narrow phone. (The phase-2 settings sheet edits the id list.)
  const toolbarIds = useComposerToolbar();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [hasSelection, setHasSelection] = useState(false);
  const [discardOpen, setDiscardOpen] = useState(false);
  useConfirmEnter(discardOpen, () => onDiscard?.());
  const visibleToolbarIds = hasSelection
    ? ["undo", "redo", "bold", "italic", "code", "link"]
    : toolbarIds;

  // Portal to <body>: the composer is rendered deep inside the app's flex layout,
  // whose ancestors form stacking contexts (the bottom navbar paints over an
  // inline fixed child). A portal escapes them so the overlay truly covers the app.
  return createPortal(
    <Box
      role="dialog"
      aria-modal="true"
      aria-label="Fullscreen message editor"
      data-mobile-pager-modal="true"
      data-mobile-focus-composer="true"
      data-mobile-keyboard-open={keyboardOpen ? "true" : undefined}
      sx={{
        // `position: absolute` (not fixed): the native shell resizes the WebView for
        // the keyboard, so `body` is normal-flow at viewport height and `absolute
        // inset:0` covers the screen. iOS WebKit text interaction inside a
        // position:fixed contenteditable is historically flaky; Obsidian's editor
        // isn't in a fixed overlay either.
        position: "absolute",
        inset: 0,
        zIndex: theme.zIndex.modal,
        bgcolor: "background.default",
        display: "flex",
        flexDirection: "column",
        // The removed top app bar used to consume this inset implicitly. Keep the
        // writing canvas below the Dynamic Island/status bar without restoring
        // an otherwise empty chrome row.
        pt: "env(safe-area-inset-top, 0px)",
        // Reserve the on-screen keyboard's height so the toolbar (last child) sits
        // ABOVE it (a fixed element's bottom otherwise hides under the keyboard on
        // iOS — the layout viewport doesn't shrink). When the keyboard is down,
        // fall back to the home-indicator safe inset.
        // Keep a narrow breathing gap between the composer card and the native
        // keyboard. The gap belongs outside the dock: padding either track would
        // disturb the shared 44pt action rhythm and make compact/fullscreen drift.
        // Browser/PWA needs the measured keyboard overlap. The native shell
        // already resizes WKWebView, so adding its home-indicator safe area here
        // creates a conspicuous second gap above the keyboard. In that focused
        // native state, keep only the shared quiet separation.
        pb: nativeShell
          ? keyboardOpen
            ? `${mobileComposerKeyboardGap}px`
            : `max(env(safe-area-inset-bottom, 0px), ${mobileComposerKeyboardGap}px)`
          : `calc(max(var(--kb-inset, 0px), env(safe-area-inset-bottom, 0px)) + ${mobileComposerKeyboardGap}px)`,
      }}
    >
      {onDiscard && (
        <Tooltip title="Ignore modifications">
          <IconButton
            aria-label="ignore modifications"
            onClick={act(() => setDiscardOpen(true))}
            sx={{
              position: "absolute",
              zIndex: 1,
              top: "calc(env(safe-area-inset-top, 0px) + 4px)",
              right: 8,
              width: 44,
              height: 44,
            }}
          >
            <Close />
          </IconButton>
        </Tooltip>
      )}

      {attachmentsSlot != null && (
        <Box sx={{ px: 1.5, pt: 1 }}>{attachmentsSlot}</Box>
      )}

      {/* The writing canvas — fills the space above the keyboard dock. */}
      <Box
        sx={{
          flex: 1,
          minHeight: 0,
          p: 1.5,
          display: "flex",
          flexDirection: "column",
          // ComposerEditor resolves this height through to `.cm-content`, so the
          // entire visible canvas is a real native editing target. Keep this wrapper
          // free of pointer handlers: UIKit must own long-press selection end to end.
        }}
      >
        <PlatformComposerEditor
          ref={editorRef}
          value={seed}
          // The token-free touch path is a controlled native textarea. Keep its
          // live value separate from CM6's frozen mount seed so typing cannot be
          // reset by the next React render.
          nativeValue={value}
          onChange={onChange}
          onSubmit={onSubmit}
          onSaveDraft={onSaveDraft}
          sessionId={sessionId}
          commands={commands}
          placeholder={placeholder}
          onPasteFiles={onPasteFiles}
          onSelectionChange={setHasSelection}
          borderless
          fill
          vim={vim}
          {...(onVimMode ? { onVimMode } : {})}
          onEscape={(): boolean => {
            if (onDiscard) {
              // Keep the Escape that requests confirmation separate from the
              // Escape MUI uses to close an already-open dialog.
              globalThis.requestAnimationFrame(() => setDiscardOpen(true));
            } else onCollapse();
            return true;
          }}
        />
      </Box>

      {
        /* One inset keyboard-adjacent card shared by main compose and row editing.
          Message actions stay above formatting, which is nearest the keyboard. */
      }
      <MobileComposerAccessoryDock
        mode={hasSelection ? "selection" : "insert"}
        formatActions={
          <MobileComposerFormatActions
            commandIds={visibleToolbarIds}
            editorRef={editorRef}
            onAttach={onAttach}
            onPasteImages={onPasteClipboardImages}
            onCustomize={act(() => {
              // The full-cover settings sheet intentionally ends this focus
              // session; ordinary actions in either bar leave it untouched.
              releaseMobileComposerFocus();
              setSettingsOpen(true);
            })}
          />
        }
        utilityActions={
          <>
            <MobileComposerAccessoryButton
              title="Attach file"
              onClick={act(onAttach)}
            >
              <AttachFile />
            </MobileComposerAccessoryButton>
            {showCollapse && (
              <MobileComposerAccessoryButton
                title="Save as draft"
                disabled={!sendable}
                onClick={act(onSaveDraft)}
              >
                <EditNoteOutlined />
              </MobileComposerAccessoryButton>
            )}
            {showCollapse && onSchedule && (
              <MobileComposerAccessoryButton
                title="Schedule send"
                disabled={!sendable}
                onClick={act(onSchedule)}
              >
                <Schedule />
              </MobileComposerAccessoryButton>
            )}
            {showCollapse && onForcePush && (
              <MobileComposerAccessoryButton
                title="Force push"
                color="warning"
                disabled={!sendable || !forcePushEnabled}
                onClick={(event): void => {
                  haptic();
                  onForcePush(event.currentTarget);
                }}
              >
                <Bolt />
              </MobileComposerAccessoryButton>
            )}
            {showCollapse && (
              <MobileComposerAccessoryButton
                title={submitLabel}
                color="primary"
                disabled={!sendable}
                onClick={act(onSubmit)}
              >
                {submitIcon ?? <Send />}
              </MobileComposerAccessoryButton>
            )}
          </>
        }
        fixedAction={
          <MobileComposerAccessoryButton
            title="Hide keyboard"
            preserveEditorFocus={false}
            onClick={act(() => {
              if (document.activeElement instanceof HTMLElement) {
                document.activeElement.blur();
              }
              if (showCollapse) onCollapse();
            })}
          >
            <KeyboardHide />
          </MobileComposerAccessoryButton>
        }
        primaryLabel={showCollapse ? "Collapse editor" : submitLabel}
        primaryDisabled={showCollapse ? false : !sendable}
        onPrimary={showCollapse ? act(onCollapse) : act(onSubmit)}
        primaryIcon={showCollapse ? <CloseFullscreen /> : submitIcon ??
          (submitLabel === "Done editing" ? <Check /> : <Send />)}
      />

      <ComposerToolbarSettings
        open={settingsOpen}
        onClose={(): void => setSettingsOpen(false)}
      />
      <ConfirmSheet
        open={discardOpen}
        onClose={(): void => setDiscardOpen(false)}
        title="Ignore modifications?"
        actions={
          <>
            <Button onClick={(): void => setDiscardOpen(false)}>
              Keep editing
              <Kbd keys="Esc" />
            </Button>
            <Button color="error" onClick={onDiscard}>
              Ignore modifications
              <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
            </Button>
          </>
        }
      >
        Your unsaved changes will be discarded.
      </ConfirmSheet>
    </Box>,
    document.body,
  );
}
