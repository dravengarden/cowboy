import { type ReactNode, type RefObject, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  AppBar,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Toolbar,
  Tooltip,
  useTheme,
} from "@mui/material";
import { Check, Close, CloseFullscreen, Send, Tune } from "@mui/icons-material";
import {
  PlatformComposerEditor,
  type ComposerEditorHandle,
} from "./composer/PlatformComposerEditor";
import { COMPOSER_COMMANDS_BY_ID, type ComposerCommand } from "./composerCommands";
import { useComposerToolbar } from "./composerToolbarConfig";
import { ComposerToolbarSettings } from "./ComposerToolbarSettings";
import type { AvailableCommand } from "./protocol";
import { haptic } from "./haptic";
import { FloatingActionIsland } from "./_shell";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";

// Brand-new full-screen mobile compose surface (NOT a DetentSheet): a fixed
// 100dvh overlay modeled on Obsidian's mobile note editor — a light top bar
// (collapse), a full-height CM6 + mdlive live-preview canvas, and a
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
}: {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onSaveDraft: () => void;
  onCollapse: () => void;
  onAttach: () => void;
  onPasteFiles: (files: File[]) => void;
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
}): React.JSX.Element {
  const theme = useTheme();

  // UNCONTROLLED, exactly like the inline composer: freeze the open-time text as a
  // one-shot seed and let `onChange` flow text OUT only. Passing the live
  // `value={text}` (what we used to do) makes @uiw/react-codemirror reconcile the
  // doc on EVERY keystroke — and worse, on every IME composition update — which
  // bounces the iOS caret and corrupts mid-composition pinyin ("状态错乱"). The
  // inline editor's own comment warns against `value={text}` for exactly this; the
  // fullscreen surface must follow the same rule. This component remounts on each
  // open (parent gates it behind `composeFs`), so the frozen seed is always the
  // current in-progress text at open time; on close the parent syncs it back.
  const seed = useRef(value).current;

  // Raise the keyboard on open. iOS can drop a single early focus while the
  // overlay is still painting, so focus a few times across the first frames
  // (cheap, idempotent) — the editor lands focused with the caret at the end.
  useEffect(() => {
    // The main composer focuses IN the opening user tap (flushSync) → skip the
    // programmatic timers, which would re-focus over the armed editor and could
    // disarm the iOS long-press menu. The row-edit overlay still uses them.
    if (!autoFocus) return undefined;
    const timers = [0, 120, 320].map((d) =>
      globalThis.setTimeout(() => editorRef.current?.focusEnd(), d)
    );
    return (): void => timers.forEach((t) => globalThis.clearTimeout(t));
  }, [autoFocus]);

  const act = (fn: () => void): (() => void) => () => {
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
  const runCmd = (cmd: ComposerCommand): void => {
    const ed = editorRef.current;
    if (ed) cmd.run({ editor: ed, attach: onAttach });
  };
  const visibleToolbarIds = hasSelection
    ? ["bold", "italic", "code", "link"]
    : toolbarIds;
  const actions = visibleToolbarIds
    .map((id) => COMPOSER_COMMANDS_BY_ID[id])
    .filter((c): c is ComposerCommand => c !== undefined)
    .map((cmd) => (
      <ToolBtn key={cmd.id} title={cmd.label} onClick={act(() => runCmd(cmd))}>
        {cmd.icon}
      </ToolBtn>
    ));

  // Portal to <body>: the composer is rendered deep inside the app's flex layout,
  // whose ancestors form stacking contexts (the bottom navbar paints over an
  // inline fixed child). A portal escapes them so the overlay truly covers the app.
  return createPortal(
    <Box
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
        // Reserve the on-screen keyboard's height so the toolbar (last child) sits
        // ABOVE it (a fixed element's bottom otherwise hides under the keyboard on
        // iOS — the layout viewport doesn't shrink). When the keyboard is down,
        // fall back to the home-indicator safe inset.
        pb: "max(var(--kb-inset, 0px), env(safe-area-inset-bottom, 0px))",
      }}
    >
      <AppBar
        position="static"
        color="transparent"
        elevation={0}
        sx={{
          // No bottom divider: a hard hairline on the near-white surface read as a
          // cheap seam under a bare top bar. The collapse button floats on the same
          // calm writing surface — a seamless, focused editor (Bear / iA Writer
          // style) instead of a boxed-off pale band.
          bgcolor: "background.default",
          pt: "env(safe-area-inset-top, 0px)",
        }}
      >
        <Toolbar variant="dense" sx={{ minHeight: 48, gap: 1 }}>
          {showCollapse && (
            <Tooltip title="Collapse">
              <IconButton aria-label="collapse editor" onClick={act(onCollapse)}>
                <CloseFullscreen />
              </IconButton>
            </Tooltip>
          )}
          <Box sx={{ flex: 1 }} />
          <Tooltip title={onDiscard ? "Ignore modifications" : submitLabel}>
            <span>
              <IconButton
                aria-label={onDiscard ? "ignore modifications" : submitLabel.toLowerCase()}
                color={onDiscard ? "default" : "primary"}
                disabled={onDiscard ? false : !sendable}
                onClick={act(onDiscard ? () => setDiscardOpen(true) : onSubmit)}
              >
                {onDiscard ? <Close /> : submitIcon ?? (submitLabel === "Done editing" ? <Check /> : <Send />)}
              </IconButton>
            </span>
          </Tooltip>
        </Toolbar>
      </AppBar>

      {attachmentsSlot != null && (
        <Box sx={{ px: 1.5, pt: 1 }}>{attachmentsSlot}</Box>
      )}

      {/* The writing canvas — fills the space between the top bar and the toolbar. */}
      <Box
        sx={{
          flex: 1,
          minHeight: 0,
          p: 1.5,
          display: "flex",
          flexDirection: "column",
          // NO content/line fill here. The empty long-pressable area below the text
          // is now Obsidian's `scrollPastEnd` padding (ComposerEditor, fill mode) —
          // a long-press in that padding snaps the caret to the line end and the iOS
          // Paste menu appears. A flex content-fill (the old approach) made the press
          // land mid-air on the `.cm-line` element → no menu; scrollPastEnd is the fix.
        }}
      >
        <PlatformComposerEditor
          ref={editorRef}
          value={seed}
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
            onCollapse();
            return true;
          }}
        />
      </Box>

      {/* One keyboard dock: formatting, the primary edit action, and settings
          share one row. Save is an exact-centre liquid-glass island rather than
          a second opaque footer row, so the keyboard does not consume another
          50px of the writing canvas. */}
      <Box
        data-toolbar-mode={hasSelection ? "wrap" : "insert"}
        sx={{
          display: "grid",
          gridTemplateColumns: onDiscard
            ? "minmax(0, 1fr) auto minmax(0, 1fr)"
            : "minmax(0, 1fr) auto",
          alignItems: "center",
          gap: 0.5,
          px: 1,
          py: 0.5,
          bgcolor: "transparent",
          userSelect: "none",
          WebkitUserSelect: "none",
        }}
      >
        <Stack
          direction="row"
          alignItems="center"
          spacing={0.25}
          sx={{ minWidth: 0, overflowX: "auto", flexWrap: "nowrap", scrollbarWidth: "none", "&::-webkit-scrollbar": { display: "none" } }}
        >
          {actions}
        </Stack>
        {onDiscard && (
          <FloatingActionIsland maxWidth={54}>
            <IconButton
              aria-label="Save"
              onClick={act(onSubmit)}
              color="inherit"
              sx={{
                width: 46,
                height: 46,
                justifySelf: "center",
                borderRadius: 999,
                color: "text.primary",
              }}
            >
              <Check />
            </IconButton>
          </FloatingActionIsland>
        )}
        <Box sx={{ justifySelf: "end", flexShrink: 0 }}>
          <ToolBtn title="Customize toolbar" onClick={act(() => setSettingsOpen(true))}>
            <Tune />
          </ToolBtn>
        </Box>
      </Box>

      <ComposerToolbarSettings
        open={settingsOpen}
        onClose={(): void => setSettingsOpen(false)}
      />
      <Dialog open={discardOpen} onClose={(): void => setDiscardOpen(false)} fullWidth maxWidth="xs">
        <DialogTitle>Ignore modifications?</DialogTitle>
        <DialogContent>Your unsaved changes will be discarded.</DialogContent>
        <DialogActions>
          <Button onClick={(): void => setDiscardOpen(false)}>Keep editing</Button>
          <Button color="error" onClick={onDiscard}>
            Ignore modifications
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </DialogActions>
      </Dialog>
    </Box>,
    document.body,
  );
}

// One toolbar button — fixed 40px tap target, secondary tint, glyph forced to a
// touch-friendly size (beats the global MuiIconButton override).
function ToolBtn({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: ReactNode;
}): React.JSX.Element {
  return (
    <Tooltip title={title}>
      <IconButton
        aria-label={title}
        onClick={onClick}
        sx={{
          width: 40,
          height: 40,
          flexShrink: 0,
          color: "text.secondary",
          "& .MuiSvgIcon-root": { fontSize: "1.375rem" },
        }}
      >
        {children}
      </IconButton>
    </Tooltip>
  );
}
