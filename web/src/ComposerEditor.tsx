import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";
import { Box, useTheme } from "@mui/material";
import CodeMirror, { type ReactCodeMirrorRef } from "@uiw/react-codemirror";
import {
  EditorView,
  keymap,
  placeholder as placeholderExt,
} from "@codemirror/view";
import { type Extension, Prec } from "@codemirror/state";
import {
  acceptCompletion,
  autocompletion,
  closeCompletion,
  completionKeymap,
  completionStatus,
  startCompletion,
} from "@codemirror/autocomplete";
import {
  defaultKeymap,
  history,
  historyKeymap,
  redo,
  undo,
} from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { cmTheme } from "./cmTheme";
import { livePreviewExtensions } from "./composerExtensions";
import { useComposerSourceMode } from "./composerSourceMode";
import { hasDraftMod, hasSendMod } from "./platform";
import {
  deleteEmptyCodeFenceBackward,
  deleteTokenBackward,
  tokenChipPlugin,
} from "./fileTokenWidget";
import {
  deleteImageTokenBackward,
  ensureTrailingImageLine,
  inlineImageField,
  inlineImageTheme,
  inlineImageTrailingLine,
  insertImageToken,
  registerInlineAttachment,
  refreshInlineImages,
  removeImageTokenById,
} from "./inlineImages";
import { clipboardFiles, type Attachment } from "./attachments";
import { dataTransferCarriesFiles } from "./composer/composerFileDrop";
import {
  fileCompletionSource,
  slashCompletionSource,
} from "./composerCompletions";
import type { AvailableCommand } from "./protocol";
import { createLoadedDesktopVimRuntime } from "./desktop/vim/runtimeLoader";
import {
  type VimEscapeState,
  vimEscapeBelongsToApp,
} from "./desktop/vim/vimEscapeOwnership";
import { inlineImagePasteInsertion } from "./inlineImageSelection";
import { moveCaretOffImageLine } from "./composer/inlineImageCaretPolicy";
import { mobileEmptyLineCaretRepair } from "./composer/mobileEmptyLineCaret";
import { mobileLineBreakCaretTelemetry } from "./composer/mobileLineBreakCaretTelemetry";
import { reportMobileNativePasteEvent } from "./composer/mobileNativePasteTelemetry";
import { composerInputDebugExtension } from "./composer/composerInputDebug";
import {
  markdownLinkForPastedUrl,
  normalizeClipboardText,
  pastedTextBeatsFiles,
} from "./composer/clipboardPastePolicy";
import { iosLineStartDashRepair } from "./composer/obsidianAutoPair";
import { readWebClipboard } from "./composer/webClipboard";
import { hasNativeClipboardBridge } from "./composer/clipboardPort";
import { isAppleTouchDevice } from "./keyboardGeometry";
import {
  cycleHeading,
  indentLines,
  insertCodeBlock,
  insertMarkdownLink,
  outdentLines,
  setHeading,
  toggleChecklist,
} from "./composer/markdownEditing";
import {
  inlineFormatCommand,
  linePrefixCommand,
  type MarkdownEditCommand,
  runMarkdownEdit,
} from "./composer/markdownEditingCommands";

export interface ComposerEditorSelection {
  anchor: number;
  head: number;
}

export interface ComposerEditorHandle {
  focus: () => void;
  /** Whether this exact editor currently owns browser/native input focus. */
  hasFocus: () => boolean;
  /** Reveal the current selection after a visual-viewport resize without
   * changing focus, selection, IME composition, or native input ownership. */
  revealSelection: () => void;
  /** Live document text, including keystrokes that have not reached React yet. */
  getValue: () => string;
  /** Read the logical selection before replacing one editor surface with another. */
  getSelection: () => ComposerEditorSelection;
  /** Focus this editor and restore a selection captured from the replaced surface. */
  focusSelection: (selection: ComposerEditorSelection) => void;
  /** Whether Escape may go to Cowboy chrome: no visible picker, and (with Vim)
   * plain Normal mode. */
  escapeBelongsToApp: () => boolean;
  // Focus AND place the caret at the very end of the document — used when
  // opening an existing draft/queued message for editing, so you continue from
  // where the text left off instead of with the caret stranded at the start.
  focusEnd: () => void;
  // Insert a trigger char (`/` or `@`) at the caret + open the picker — used by
  // the action-row buttons. Mirrors the old `appendToken` + focus behavior.
  insertTrigger: (ch: string) => void;
  // Replace the current or captured logical range with literal clipboard text.
  insertText: (text: string, selection?: ComposerEditorSelection) => void;
  // Insert an image at the caret as an inline `![](cowboy-att:id)` token (the host
  // adds the bytes to `attachments[]`; this renders it as an inline thumbnail).
  insertImage: (a: Attachment) => void;
  // Batch paste is one document transaction. Inserting files one-by-one can
  // reuse a stale native-textarea value during the touch → CM6 promotion.
  insertImages: (
    attachments: Attachment[],
    selection?: ComposerEditorSelection,
  ) => void;
  // Rebuild image widgets after an asynchronously encoded paste replaces its
  // same-id object-URL placeholder with durable data.
  refreshImages: () => void;
  // Remove a specific inline image (by id) from the doc — the selection popover's
  // Delete action.
  deleteImage: (id: string) => void;
  // Clear the document imperatively. Submit can't rely on the controlled
  // `value=""` prop to empty the editor: @uiw/react-codemirror (≥4.24) holds a
  // 200ms "typing latch" and DEFERS external value-prop changes while you're
  // still within 200ms of your last keystroke (an IME-echo guard). Hitting
  // Cmd-Enter right after typing lands inside that window, so the prop-driven
  // clear gets parked until the latch expires — the text lingers after the
  // message already sent. Dispatching straight to the view bypasses the latch
  // and clears now.
  clear: () => void;
  // Returns one command only when its slash completion was explicitly selected.
  // Typed `/`, `/dir`, and paths deliberately have no command intent.
  consumeSelectedSlashCommand: () => string | null;
  // Markdown toolbar actions (the fullscreen keyboard toolbar). Both editor
  // engines apply the same Obsidian command implementations
  // (composer/markdownEditing.ts): CM6 dispatches one transaction, while the
  // native touch textarea applies one undoable edit and keeps UIKit selection.
  /// Wrap the selection (or insert the marker pair at the caret) — bold `**`,
  /// italic `*`, inline code `` ` ``.
  wrap: (before: string, after: string) => void;
  /// Obsidian's formatting toggle for the marker's format — bold `**`, italic
  /// `*`, strikethrough `~~`, highlight `==`, code `` ` ``, math `$`, comment
  /// `%%`: wraps the word at a caret, removes an enclosing span, or steps over
  /// the closing marker.
  toggleWrap: (marker: string) => void;
  /// Obsidian's quote-aware indent / outdent of every selected line.
  indent: () => void;
  outdent: () => void;
  /// Obsidian's list/quote toggle for `- `, `1. `, `- [ ] ` or `> ` on every
  /// selected line (converting between list kinds).
  toggleLinePrefix: (prefix: string) => void;
  /// Cycle the heading level of the selected lines: none → 1 → 2 → 3 → none.
  cycleHeading: () => void;
  /// Set the selected lines' heading to an exact level (1–6); `0` removes it.
  setHeading: (level: number) => void;
  /// Obsidian's checklist status cycle: plain/list → `[ ]` → `[x]` → `[ ]`.
  toggleCheckbox: () => void;
  /// Obsidian's link insert: `[|]()`, or `[selection](|)`.
  insertLink: () => void;
  /// Wrap the selected lines in a fenced ``` code block.
  insertCodeBlock: () => void;
  /// Undo / redo (the toolbar's history buttons).
  undo: () => void;
  redo: () => void;
}

// Just above @codemirror/autocomplete's default 75ms interactionDelay.
const COMPLETION_ACCEPT_DELAY_MS = 90;

function completionListVisible(view: EditorView): boolean {
  return view.dom.querySelector(".cm-tooltip-autocomplete") !== null;
}

function revealFocusedSelection(view: EditorView): void {
  if (!view.hasFocus || !view.dom.isConnected) return;
  view.dispatch({
    effects: EditorView.scrollIntoView(view.state.selection.main.head, {
      y: "nearest",
      yMargin: 12,
    }),
  });
}

// Reads the actual Vim state from the loaded module's CM5-compat handle. Escape
// stays with Vim through Insert, Visual, and operator/key-prefix states; only
// plain Normal delegates to Cowboy chrome. The same handle also drives the
// NORMAL/INSERT hint surfaced in the composer card.
type VimModeEvent = { mode?: string; subMode?: string };
type CmVimHandle = {
  state?: { vim?: VimEscapeState };
  on?: (event: "vim-mode-change", handler: (e: VimModeEvent) => void) => void;
  off?: (event: "vim-mode-change", handler: (e: VimModeEvent) => void) => void;
};
type VimApi = {
  getCM: (view: EditorView) => CmVimHandle | null;
};

// Backspace on an EMPTY symmetric markdown marker pair deletes BOTH sides at
// once — Obsidian's "delete the front and the back goes too", extended to the
// MULTI-char markers (`**`, `~~`, `==`). cowboy's closeBrackets only knows the
// single-char pairs (`*`, `_`, `` ` ``), so an empty `~~|~~` / `**|**` / `==|==`
// (what the toggle toolbar inserts) otherwise needs two presses and deletes
// asymmetrically. Longest marker first so `**|**` clears the full `**`, not one
// `*`. No-op (false) → the normal Backspace chain runs.
const EMPTY_PAIR_MARKERS = ["**", "~~", "==", "`", "*", "_"];
function deleteEmptyMarkerPairBackward(view: EditorView): boolean {
  const { state } = view;
  const r = state.selection.main;
  if (!r.empty) return false;
  const pos = r.head;
  for (const m of EMPTY_PAIR_MARKERS) {
    const k = m.length;
    if (pos - k < 0 || pos + k > state.doc.length) continue;
    if (state.sliceDoc(pos - k, pos) === m && state.sliceDoc(pos, pos + k) === m) {
      view.dispatch({
        changes: { from: pos - k, to: pos + k },
        selection: { anchor: pos - k },
        userEvent: "delete.backward",
      });
      return true;
    }
  }
  return false;
}

// The custom Backspace chain, by SPECIFICITY: empty marker pair → inline-image
// token → empty code fence → @/​/ token. Each no-ops (false) when it doesn't
// apply, so order is safe; returns true once one consumes the delete. Shared by
// BOTH delete channels — the keymap (physical keyboard `keydown`) and the
// beforeinput handler (phone soft keyboards, which emit no Backspace keydown).
// Inline-image sits after the @-token deliberately: its token contains spaces,
// so the @-token regex can't match it.
function backspaceChain(view: EditorView): boolean {
  return deleteEmptyMarkerPairBackward(view) ||
    deleteImageTokenBackward(view) ||
    deleteEmptyCodeFenceBackward(view) ||
    deleteTokenBackward(view);
}

// Desktop-only Vim. PlatformComposerEditor preloads the composition-aware
// runtime before mounting an interactive editor. Creating it synchronously here
// is intentional: adding Vim to an already-editable CM6 instance dispatches an
// extension reconfiguration that can invalidate macOS native marked text.
function useVimExtension(
  enabled: boolean,
  apiRef: { current: VimApi | null },
): Extension | null {
  const runtime = useMemo(() => {
    const desktop = typeof window !== "undefined" &&
      window.matchMedia("(pointer: fine) and (hover: hover)").matches;
    return enabled && desktop ? createLoadedDesktopVimRuntime() : null;
  }, [enabled]);
  apiRef.current = runtime
    ? { getCM: runtime.getCM as VimApi["getCM"] }
    : null;
  return runtime?.extension ?? null;
}

// CodeMirror-6 composer input, styled as a MUI outlined field. Replaces the
// `<textarea>` (which forced the iOS keyboard Form Assistant bar) and folds the
// old Popper `@`/`/` pickers into CM autocomplete. (Plan Steps 5-9, 11.)
export const ComposerEditor = forwardRef<
  ComposerEditorHandle,
  {
    value: string;
    onChange: (value: string) => void;
    onSubmit: () => void;
    // ⌃⏎ (mac) / Alt+⏎ — park the current text as a draft instead of sending.
    onSaveDraft?: () => void;
    // Fired when the send chord (⌘⏎) is HELD past the long-press threshold while
    // `holdToForce` is set (i.e. the session is busy) — the keyboard analog of
    // holding the Queue button. Opens the force-push confirm.
    onForceHold?: () => void;
    // When true (session busy/starting) the send chord distinguishes a tap
    // (queue, fired on keyup) from a hold (force, fired by the timer). When false
    // (idle) the send chord fires onSubmit instantly on keydown — zero latency on
    // the hot path, no hold semantics.
    holdToForce?: boolean;
    sessionId: string;
    commands: () => AvailableCommand[];
    placeholder?: string;
    disabled?: boolean;
    autoFocus?: boolean;
    /** Initial caret for the one native-textarea -> CM6 promotion mount. */
    initialSelection?: number;
    /** Touch CM6 exists only for inline-image widgets; enables iOS caret repair. */
    touchInput?: boolean;
    vim?: boolean;
    /// Called when the vim mode changes (normal / insert / visual). Drives the
    /// NORMAL/INSERT hint in the composer card. Only wired when vim is on.
    onVimMode?: (mode: string) => void;
    // Called on Escape when it should act on the app, not the editor: Vim off,
    // or Vim on in plain Normal mode. Insert, Visual, operator-pending, and
    // partial prefixes normalize through Vim first, so a later plain-Normal
    // Escape may unwind an active surrounding edit/layer. Returns true when consumed.
    onEscape?: () => boolean;
    // Called with image / file blobs found on a clipboard paste (a screenshot,
    // a copied image). When it handles them the editor swallows the paste so
    // no stray base64 / filename text lands in the document.
    onPasteFiles?: (files: File[]) => void;
    /// Fired on every selection/doc change with whether a non-empty range is
    /// selected — drives the fullscreen keyboard toolbar's insert↔wrap action swap.
    onSelectionChange?: (hasSelection: boolean) => void;
    /// Right padding (px) reserved on the editor content so text never runs under
    /// the action buttons the composer overlays at the input's bottom-right.
    endInset?: number;
    /// Drop the editor's own notched-outline border + hover/focus ring. Used when
    /// the editor sits INSIDE the composer's outlined Paper card (the card owns the
    /// box) — without this the card border + the editor border would double up.
    borderless?: boolean;
    /// Expanded mode: a tall fixed editing area (~50vh) instead of the compact
    /// auto-grow (≤40vh). The Zed-style ↗ toggle in the card drives this.
    expanded?: boolean;
    /// Drag-resized expanded height (px). 0 → the 48vh default. Only honoured when
    /// `expanded`; the top-edge resize handle writes it (composerExpand store).
    heightPx?: number;
    /// Fill mode (two-column desktop layout): the editor stretches to fill its
    /// flex parent's height instead of the vh-bounded compact/expanded sizes. The
    /// column height IS the size, so there's no compact↔expand toggle here.
    /// Overrides `expanded`/`heightPx`.
    fill?: boolean;
    /// Desktop column layout: keep CodeMirror's vertical scrollbar flush with
    /// the card edge while reserving the overlaid action gutter on content only.
    /// Touch surfaces deliberately retain their established inset scroller.
    flushRightScrollbar?: boolean;
  }
>(function ComposerEditor(
  {
    value,
    onChange,
    onSubmit,
    onSaveDraft,
    onForceHold,
    holdToForce,
    sessionId,
    commands,
    placeholder,
    disabled,
    autoFocus = false,
    initialSelection,
    touchInput = false,
    vim,
    onVimMode,
    onEscape,
    onPasteFiles,
    onSelectionChange,
    endInset = 0,
    borderless = false,
    expanded = false,
    heightPx = 0,
    fill = false,
    flushRightScrollbar = false,
  },
  ref,
): React.JSX.Element {
  const theme = useTheme();
  // The fixed expanded height: the drag-resized px if set, else the 48vh default.
  // Clamp via CSS so a px persisted on a taller viewport can't overflow a short one.
  const expandedHeight = heightPx > 0 ? `min(${String(heightPx)}px, 82vh)` : "48vh";
  const cmRef = useRef<ReactCodeMirrorRef>(null);
  const selectedSlashCommandRef = useRef<string | null>(null);
  const acceptCompletionWhenReadyRef = useRef(false);
  // Keep latest callbacks/data in refs so the (memoized) extensions never go
  // stale without rebuilding the editor state on every keystroke.
  const onSubmitRef = useRef(onSubmit);
  onSubmitRef.current = onSubmit;
  const onSaveDraftRef = useRef(onSaveDraft);
  onSaveDraftRef.current = onSaveDraft;
  const onForceHoldRef = useRef(onForceHold);
  onForceHoldRef.current = onForceHold;
  const holdToForceRef = useRef(holdToForce);
  holdToForceRef.current = holdToForce;
  const onSelectionChangeRef = useRef(onSelectionChange);
  onSelectionChangeRef.current = onSelectionChange;
  // @uiw/react-codemirror includes `onChange` in the dependency list of the
  // effect that dispatches StateEffect.reconfigure. Parent composer state can
  // legitimately produce a new callback during typing, but passing that
  // identity through would reconfigure the whole CM6 state after every input
  // transaction — including in the middle of native IME marked text. Keep one
  // bridge for the editor lifetime and route the latest product callback
  // through a ref, just like every other callback captured by `extensions`.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const handleChange = useCallback((next: string): void => {
    onChangeRef.current(next);
  }, []);
  // Long-press-send timing. `holdTimer` is armed on the first send-chord keydown
  // while busy; if it survives to the threshold it opens the force confirm
  // (`forceFired` guards against the keyup then also queuing). 450ms matches the
  // Queue button's hold.
  const holdTimer = useRef<number | undefined>(undefined);
  const forceFired = useRef(false);
  const commandsRef = useRef(commands);
  commandsRef.current = commands;
  const onEscapeRef = useRef(onEscape);
  onEscapeRef.current = onEscape;
  const onPasteFilesRef = useRef(onPasteFiles);
  onPasteFilesRef.current = onPasteFiles;
  // Set synchronously by useVimExtension on the first interactive Desktop
  // mount; read by the Escape keymap to tell insert from normal/visual mode.
  const vimApiRef = useRef<VimApi | null>(null);

  const vimExt = useVimExtension(vim ?? false, vimApiRef);

  // Source mode is read here, not prop-drilled: the compact composer, the
  // fullscreen editor and the queue/draft editors are separate mounts of THIS
  // component, and they must never disagree about how the same markdown is
  // presented. A flip changes the memo below, which @uiw/react-codemirror
  // applies as a StateEffect.reconfigure — the document, selection and undo
  // history are CM6 state, not configuration, so they all survive.
  const sourceMode = useComposerSourceMode();

  // Surface the live vim mode to the card's NORMAL/INSERT hint. Once Vim is
  // present (vimExt truthy → vimApiRef populated), subscribe to the
  // CM5-compat `vim-mode-change` event and emit the initial mode. No-op when vim
  // is off or not yet loaded; cleans up on toggle/unmount.
  const onVimModeRef = useRef(onVimMode);
  onVimModeRef.current = onVimMode;
  useEffect(() => {
    if (!(vim ?? false) || !vimExt) return undefined;
    const view = cmRef.current?.view;
    const cm = view ? vimApiRef.current?.getCM(view) : null;
    if (!cm?.on) return undefined;
    const handler = (e: VimModeEvent): void => {
      onVimModeRef.current?.(e.mode ?? "normal");
    };
    cm.on("vim-mode-change", handler);
    onVimModeRef.current?.(cm.state?.vim?.insertMode ? "insert" : "normal");
    return (): void => cm.off?.("vim-mode-change", handler);
  }, [vim, vimExt]);

  // A held send-chord that unmounts mid-press must not leave its timer running.
  useEffect(() => (): void => {
    if (holdTimer.current !== undefined) {
      globalThis.clearTimeout(holdTimer.current);
    }
  }, []);

  // On touch devices the composer is pinned to the bottom edge and the on-screen
  // keyboard overlays the layout viewport WITHOUT shrinking it. CM measures space
  // against the layout viewport, so it sees room "below" the cursor (the area the
  // keyboard now covers) and renders the `@`/`/` picker downward — hidden behind
  // the keyboard. Forcing `aboveCursor` flips the picker up, where there's always
  // room (the composer is at the bottom). Desktop keeps CM's default auto-flip.
  const aboveCursor = useMemo(
    () =>
      typeof window !== "undefined" &&
      window.matchMedia("(pointer: coarse)").matches,
    [],
  );

  const runViewCommand = (
    command: MarkdownEditCommand,
    userEvent?: string,
  ): void => {
    const view = cmRef.current?.view;
    if (!view) return;
    runMarkdownEdit(view, command, userEvent);
    view.focus();
  };

  useImperativeHandle(ref, () => ({
    focus: (): void => cmRef.current?.view?.focus(),
    hasFocus: (): boolean => cmRef.current?.view?.hasFocus ?? false,
    revealSelection: (): void => {
      const view = cmRef.current?.view;
      if (view) revealFocusedSelection(view);
    },
    getValue: (): string => cmRef.current?.view?.state.doc.toString() ?? "",
    getSelection: (): ComposerEditorSelection => {
      const selection = cmRef.current?.view?.state.selection.main;
      return selection
        ? { anchor: selection.anchor, head: selection.head }
        : { anchor: 0, head: 0 };
    },
    focusSelection: (selection: ComposerEditorSelection): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      const clamp = (position: number): number =>
        Math.max(0, Math.min(position, view.state.doc.length));
      view.dispatch({
        selection: {
          anchor: clamp(selection.anchor),
          head: clamp(selection.head),
        },
        scrollIntoView: true,
      });
      view.focus();
    },
    escapeBelongsToApp: (): boolean => {
      const view = cmRef.current?.view;
      // A visible completion list owns Escape first, as Obsidian's suggest
      // does; a surrounding capture must not open its discard dialog.
      if (view && completionListVisible(view)) return false;
      // A pending or fully filtered query has nothing on screen. The app is
      // about to own this Escape, so end the query now; otherwise it could
      // open its list over the app's dialog.
      if (view && completionStatus(view.state) !== null) closeCompletion(view);
      const state = view
        ? vimApiRef.current?.getCM(view)?.state?.vim
        : undefined;
      return vimEscapeBelongsToApp(vim ?? false, state);
    },
    focusEnd: (): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      const end = view.state.doc.length;
      view.dispatch({ selection: { anchor: end } });
      view.focus();
    },
    insertTrigger: (ch: string): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      const pos = view.state.selection.main.head;
      view.dispatch({
        changes: { from: pos, insert: ch },
        selection: { anchor: pos + ch.length },
      });
      view.focus();
      startCompletion(view);
    },
    insertText: (
      text: string,
      capturedSelection?: ComposerEditorSelection,
    ): void => {
      const view = cmRef.current?.view;
      // CM6 splits "\r\n" into one line break, so a raw CRLF length would put
      // the caret past the inserted text (or past the document end, throwing).
      const insert = normalizeClipboardText(text);
      if (!view || insert.length === 0) return;
      const selection = capturedSelection ?? view.state.selection.main;
      const clamp = (position: number): number =>
        Math.max(0, Math.min(position, view.state.doc.length));
      const from = Math.min(clamp(selection.anchor), clamp(selection.head));
      const to = Math.max(clamp(selection.anchor), clamp(selection.head));
      const caret = from + insert.length;
      view.dispatch({
        changes: { from, to, insert },
        selection: { anchor: caret },
        scrollIntoView: true,
        // Never join a paste with adjacent typing in one undo step.
        userEvent: "input.paste",
      });
      view.focus();
    },
    insertImage: (a: Attachment): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      insertImageToken(view, a);
    },
    insertImages: (
      attachments: Attachment[],
      capturedSelection?: ComposerEditorSelection,
    ): void => {
      const view = cmRef.current?.view;
      if (!view || attachments.length === 0) return;
      attachments.forEach(registerInlineAttachment);
      const selection = capturedSelection ?? view.state.selection.main;
      const edit = inlineImagePasteInsertion(
        view.state.doc.toString(),
        selection.anchor,
        selection.head,
        attachments,
      );
      view.dispatch({
        changes: { from: edit.from, to: edit.to, insert: edit.insert },
        selection: { anchor: edit.caret },
        scrollIntoView: true,
        userEvent: "input.paste",
      });
      view.focus();
    },
    refreshImages: (): void => {
      cmRef.current?.view?.dispatch({ effects: refreshInlineImages.of(null) });
    },
    deleteImage: (id: string): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      removeImageTokenById(view, id);
    },
    clear: (): void => {
      selectedSlashCommandRef.current = null;
      const view = cmRef.current?.view;
      if (!view) return;
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: "" },
      });
      // iOS repaint nudge. A keystroke makes WebKit repaint the contenteditable,
      // but this PROGRAMMATIC empty doesn't — so after Send the just-sent text
      // lingers on screen even though the doc is now empty (the .cm-content
      // compositing layer isn't re-rasterized). Toggling opacity for one frame
      // forces a repaint of the (now empty) content. Focus-preserving and
      // layout-neutral, so the keyboard stays up and nothing reflows. No-op cost
      // on desktop.
      const content = view.contentDOM;
      content.style.opacity = "0.999";
      requestAnimationFrame(() => {
        content.style.opacity = "";
      });
    },
    consumeSelectedSlashCommand: (): string | null => {
      const command = selectedSlashCommandRef.current;
      selectedSlashCommandRef.current = null;
      return command;
    },
    wrap: (before: string, after: string): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      const { from, to } = view.state.selection.main;
      const sel = view.state.sliceDoc(from, to);
      view.dispatch({
        changes: { from, to, insert: before + sel + after },
        // Empty selection → caret between the markers (start typing inside).
        // Non-empty → re-select the wrapped text so a second tap can toggle.
        selection: from === to
          ? { anchor: from + before.length }
          : {
            anchor: from + before.length,
            head: from + before.length + sel.length,
          },
      });
      view.focus();
    },
    // Toolbar Markdown commands share Obsidian's pure implementations with
    // the native touch textarea (composer/markdownEditing.ts), so both engines
    // produce the same document for the same tap.
    toggleWrap: (marker: string): void => {
      const command = inlineFormatCommand(marker);
      if (command) runViewCommand(command);
    },
    indent: (): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      runViewCommand((doc, selection) =>
        indentLines(doc, selection, view.state.facet(indentUnit))
      , "input.indent");
    },
    outdent: (): void => {
      const view = cmRef.current?.view;
      if (!view) return;
      runViewCommand((doc, selection) =>
        outdentLines(
          doc,
          selection,
          view.state.facet(indentUnit),
          view.state.tabSize,
        )
      , "delete.dedent");
    },
    toggleLinePrefix: (prefix: string): void => {
      const command = linePrefixCommand(prefix);
      if (command) runViewCommand(command);
    },
    cycleHeading: (): void => runViewCommand(cycleHeading),
    setHeading: (level: number): void =>
      runViewCommand((doc, selection) => setHeading(doc, selection, level)),
    toggleCheckbox: (): void => runViewCommand(toggleChecklist),
    insertLink: (): void => runViewCommand(insertMarkdownLink),
    insertCodeBlock: (): void => runViewCommand(insertCodeBlock),
    undo: (): void => {
      const view = cmRef.current?.view;
      if (view) {
        undo(view);
        view.focus();
      }
    },
    redo: (): void => {
      const view = cmRef.current?.view;
      if (view) {
        redo(view);
        view.focus();
      }
    },
  }));

  const extensions = useMemo<Extension[]>(
    () => [
      // Do NOT add a pointerdown handler that places the caret at the doc end on an
      // empty-area press: dispatching a selection change at pointerdown CANCELS iOS's
      // in-progress long-press recognizer, so the empty-area Paste menu never opens
      // (it then only worked when long-pressing directly ON the placeholder/text).
      // Fill mode instead resolves the full height chain to `.cm-content`, making
      // the visible blank canvas the native contenteditable hit target. Do not use
      // `scrollPastEnd()` here: its viewport-sized padding looks editable but is an
      // unreliable UIKit menu anchor away from real lines.
      EditorView.lineWrapping,
      // Publish selection-empty state to the fullscreen keyboard toolbar so it can
      // swap insert↔wrap actions. Ref-routed so the memo never rebuilds for it.
      EditorView.updateListener.of((u): void => {
        if (u.docChanged && selectedSlashCommandRef.current) {
          const command = selectedSlashCommandRef.current;
          const text = u.state.doc.toString();
          const rest = text.slice(command.length + 1);
          if (!text.startsWith(`/${command}`) || (rest !== "" && !/^\s/u.test(rest))) {
            selectedSlashCommandRef.current = null;
          }
        }
        if (u.selectionSet || u.docChanged) {
          onSelectionChangeRef.current?.(!u.state.selection.main.empty);
        }
        if (acceptCompletionWhenReadyRef.current) {
          const status = completionStatus(u.state);
          if (u.docChanged || u.selectionSet || status === null) {
            acceptCompletionWhenReadyRef.current = false;
          } else if (status === "active") {
            acceptCompletionWhenReadyRef.current = false;
            // Outside the update cycle, after CM6's interaction delay for a
            // freshly opened list.
            const view = u.view;
            globalThis.setTimeout(() => {
              if (completionStatus(view.state) === "active") {
                acceptCompletion(view);
              }
            }, COMPLETION_ACCEPT_DELAY_MS);
          }
        }
      }),
      // Clipboard paste of image / file blobs (a screenshot, a copied image)
      // is lifted out to the composer as attachments; only a files-bearing
      // paste is swallowed, so plain-text paste keeps CodeMirror's behaviour.
      EditorView.domEventHandlers({
        click: (_event, view): boolean => {
          if (!touchInput || !view.hasFocus) return false;
          // The keyboard may already be open, so the outer false->true keyboard
          // transition cannot reveal a newly tapped caret. Wait until WebKit and
          // CM6 have committed the native click selection, then scroll only the
          // editor viewport. This does not prevent the click, rewrite selection,
          // or observe pointerdown, so UIKit keeps long-press and IME ownership.
          globalThis.requestAnimationFrame(() => revealFocusedSelection(view));
          return false;
        },
        // Obsidian's paste order: rich text (unless its HTML is only the copied
        // picture) → a URL over a selection becomes a Markdown link → files →
        // plain text.
        paste: (event, view): boolean => {
          const cb = event.clipboardData;
          if (!cb) return false;
          const text = cb.getData("text/plain") || cb.getData("text/uri-list");
          const files = pastedTextBeatsFiles(cb) ? [] : clipboardFiles(cb);
          const consumesFiles = files.length > 0 && !!onPasteFilesRef.current;
          if (touchInput) {
            reportMobileNativePasteEvent({
              surface: "cm6",
              clipboard: cb,
              fileCount: files.length,
              consumed: consumesFiles,
            });
          }
          const main = view.state.selection.main;
          const link = files.length === 0 && !view.composing
            ? markdownLinkForPastedUrl(
              view.state.sliceDoc(main.from, main.to),
              text,
            )
            : null;
          if (link !== null) {
            event.preventDefault();
            view.dispatch({
              changes: { from: main.from, to: main.to, insert: link },
              selection: { anchor: main.from + link.length },
              scrollIntoView: true,
              userEvent: "input.paste",
            });
            return true;
          }
          if (consumesFiles) {
            event.preventDefault();
            onPasteFilesRef.current?.(files);
            return true;
          }
          if (text !== "" || files.length > 0) return false;
          // CM6's stock paste replaces the selection with the empty string
          // when a payload carries nothing readable (an iOS keyboard-shelf
          // photo often arrives as an empty DataTransfer). Keep the selection;
          // a touch browser/PWA may still read an image from the same gesture,
          // as the native textarea does. The native shell has its own bridge.
          event.preventDefault();
          const onPasteFiles = onPasteFilesRef.current;
          if (touchInput && onPasteFiles && !hasNativeClipboardBridge()) {
            void readWebClipboard().then((contents) => {
              if (contents.files.length > 0) onPasteFiles(contents.files);
            });
          }
          return true;
        },
        // Desktop file drop takes the paste path at the drop point. CM6's
        // default would read every dropped file as text into the document.
        // Preventing the default tells a surrounding card drop target that
        // the drop is already attached.
        drop: (event, view): boolean => {
          if (
            touchInput || !onPasteFilesRef.current ||
            !dataTransferCarriesFiles(event.dataTransfer)
          ) return false;
          event.preventDefault();
          const files = clipboardFiles(event.dataTransfer!);
          if (files.length === 0) return true;
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          if (pos !== null) view.dispatch({ selection: { anchor: pos } });
          view.focus();
          onPasteFilesRef.current(files);
          return true;
        },
        // NOTE: the iOS IME composition "dance" (drop the .cm-scroller translateZ
        // layer on compositionstart, opacity-nudge on update/end, self-heal on
        // blur) lived here ONLY to serve the PWA's translateZ repaint hack — which
        // existed only because the PWA locked the body position:fixed. The native
        // shell runs in normal flow, so there is no repaint bug, no translateZ
        // layer, and nothing to fight: native IME / caret / paste work directly.
        // Removed at the root (PWA mobile path retired). Do NOT re-add.
      }),
      history(),
      placeholderExt(placeholder ?? ""),
      // Monospace + the Zed block-cursor styling only when vim is active (the
      // "code editor" mode); normal chat keeps the prose font. vimExt is already
      // a dep of this memo, so toggling vim rebuilds the theme.
      cmTheme(theme, !!vimExt),
      tokenChipPlugin,
      // Obsidian-style images: token stays on a real `.cm-line`; thumbnail
      // hangs below it. See inlineImages.ts.
      inlineImageField,
      inlineImageTheme,
      inlineImageTrailingLine,
      ...(touchInput
        ? [mobileEmptyLineCaretRepair, mobileLineBreakCaretTelemetry]
        : []),
      composerInputDebugExtension(touchInput ? "mobile" : "desktop"),
      // CM6 defaults the editable to spellcheck/autocorrect/autocapitalize
      // off. Obsidian turns all three on, and the touch composer's native
      // textarea has them on by default, so promoting a message to CM6 used to
      // silently drop QuickType, autocapitalization and double-space period
      // mid-message. Constant for the editor lifetime: changing attributes
      // later is a reconfigure, which must never span native composition.
      // Desktop keeps OS autocorrect off (WKWebView honors it, unlike
      // Obsidian's Electron) so paths and commands are never rewritten.
      EditorView.contentAttributes.of(
        touchInput
          ? { spellcheck: "true", autocorrect: "on", autocapitalize: "on" }
          : { spellcheck: "true" },
      ),
      ...(touchInput && isAppleTouchDevice(globalThis.navigator ?? {})
        ? [iosLineStartDashRepair]
        : []),
      // Obsidian's EditorSuggest owns Enter/Tab/arrows for as long as its list
      // is on screen. CM6 keeps a stale list visible but disabled while an
      // async `@file` query refreshes, and completionKeymap then lets Enter
      // fall through to a newline (or ArrowDown move the caret). Tab accepts,
      // as it does in Obsidian's link suggest, instead of leaving the editor.
      Prec.highest(keymap.of([
        {
          // Obsidian's Enter picks the highlighted suggestion even while its
          // list refreshes. CM6 cannot accept a disabled list, so accept as
          // soon as the refreshed list arrives (see the update listener).
          key: "Enter",
          run: (view: EditorView): boolean => {
            if (
              completionStatus(view.state) !== "pending" ||
              !completionListVisible(view)
            ) return false;
            acceptCompletionWhenReadyRef.current = true;
            return true;
          },
        },
        ...["ArrowUp", "ArrowDown"].map((key) => ({
          key,
          run: (view: EditorView): boolean =>
            completionStatus(view.state) === "pending" &&
            completionListVisible(view),
        })),
        {
          key: "Tab",
          run: (view: EditorView): boolean =>
            acceptCompletion(view) ||
            (completionStatus(view.state) === "pending" &&
              completionListVisible(view)),
        },
        {
          // Escape closes a visible list only. A pending or fully filtered
          // query has nothing on screen, so close it silently and let the
          // same Escape reach Cowboy's surface (collapse, discard, stop).
          key: "Escape",
          run: (view: EditorView): boolean => {
            if (completionStatus(view.state) === null) return false;
            const visible = completionListVisible(view);
            closeCompletion(view);
            return visible;
          },
        },
      ])),
      autocompletion({
        override: [
          fileCompletionSource(sessionId),
          slashCompletionSource(
            () => commandsRef.current(),
            (command) => {
              selectedSlashCommandRef.current = command;
            },
          ),
        ],
        activateOnTyping: true,
        icons: false,
        aboveCursor,
      }),
      // Modified-Enter chords. Handled as raw DOM events (not a CM keymap) so we
      // get keyUP + the OS auto-repeat flag — needed to tell a send TAP from a
      // long-press FORCE. Plain Enter is untouched here, so the completion picker
      // and newline behaviour fall through to the keymaps below.
      //   ⌘⏎ (send chord): idle → submit instantly on keydown; busy → start a
      //     hold timer, fire force at the threshold, else queue on keyup.
      //   ⌃⏎ / Alt+⏎ (draft chord): save the current text as a draft.
      Prec.highest(
        EditorView.domEventHandlers({
          keydown: (e, view): boolean => {
            // CM6 already withholds key events during composition (and for
            // Safari's post-compositionend Enter), so like Obsidian this
            // handler only checks the live composition state. An idle macOS
            // CJK input source still labels a physical ⌘⏎ as keyCode 229 /
            // `Process` (pitfall #96); that is a chord, not IME input, and
            // treating it as IME fell through to defaultKeymap's Mod-Enter
            // blank line instead of sending.
            const enter = e.key === "Enter" ||
              ((e.key === "Process" || e.keyCode === 229) &&
                (e.code === "Enter" || e.code === "NumpadEnter") &&
                (hasDraftMod(e) || hasSendMod(e)));
            if (
              !enter || e.shiftKey || e.isComposing || view.composing
            ) return false;
            if (hasDraftMod(e)) {
              e.preventDefault();
              onSaveDraftRef.current?.();
              return true;
            }
            if (!hasSendMod(e)) return false;
            e.preventDefault();
            if (!holdToForceRef.current) {
              // Idle: instant send. Ignore auto-repeats from a held key.
              if (!e.repeat) onSubmitRef.current();
              return true;
            }
            // Busy: the first press arms the long-press timer; repeats are ignored
            // (the timer, not the repeat, decides). Tap vs hold resolves on keyup.
            if (e.repeat) return true;
            forceFired.current = false;
            if (holdTimer.current !== undefined) {
              globalThis.clearTimeout(holdTimer.current);
            }
            holdTimer.current = globalThis.setTimeout(() => {
              holdTimer.current = undefined;
              forceFired.current = true;
              onForceHoldRef.current?.();
            }, 450);
            return true;
          },
          keyup: (e): boolean => {
            // Releasing the chord before the threshold is a TAP → queue. We watch
            // the MODIFIER keyup (Meta/Control), not just Enter: macOS suppresses a
            // key's keyup while ⌘ is held, so the Enter keyup may never arrive — but
            // the ⌘ (Meta) keyup always does. Without this every ⌘⏎ tap would sit
            // until the timer fired and wrongly open the force confirm.
            if (e.key !== "Enter" && e.key !== "Meta" && e.key !== "Control") {
              return false;
            }
            if (holdTimer.current !== undefined) {
              globalThis.clearTimeout(holdTimer.current);
              holdTimer.current = undefined;
              if (!forceFired.current) onSubmitRef.current();
            }
            return false;
          },
        }),
      ),
      // Physical-keyboard path: a real `keydown` drives the chain via the keymap.
      Prec.high(keymap.of([
        {
          key: "Backspace",
          run: (view): boolean => view.composing ? false : backspaceChain(view),
        },
        ...(touchInput
          ? [{ key: "Enter", run: moveCaretOffImageLine }]
          : []),
      ])),
      // No `beforeinput` Backspace channel, like Obsidian. CM6 already routes a
      // soft-keyboard Backspace/Enter to keymaps: iOS fires a real keydown that
      // CM6 parks as `pendingIOSKey` and replays into the keymap once the
      // native edit mutates the DOM (Android Chrome: `delayAndroidKey` from
      // beforeinput). Consuming the beforeinput here left that parked key
      // alive, and CM6's 250ms fallback replayed it as a second Backspace:
      // one soft-keyboard press deleted an @-token plus the character before
      // it, and skipped the image ring confirmation (PITFALLS #12, #109).
      // Escape belongs to Vim until it reaches plain Normal mode: Insert exits
      // to Normal, Visual clears its selection, and pending operators/prefixes
      // cancel. Only a later Normal-mode Escape may reach Cowboy's active
      // surrounding transaction. With Vim off, Cowboy keeps its direct Escape behavior.
      // High precedence keeps the ownership decision ahead of defaultKeymap's
      // clear-selection Escape.
      Prec.high(
        keymap.of([
          {
            key: "Escape",
            run: (view): boolean => {
              const state = vimApiRef.current?.getCM(view)?.state?.vim;
              if (!vimEscapeBelongsToApp(vim ?? false, state)) return false;
              return onEscapeRef.current?.() ?? false;
            },
          },
        ]),
      ),
      // completionKeymap first so Enter/Tab/arrows drive the picker when it's
      // open, falling through to newline/normal editing when it's closed.
      keymap.of([...completionKeymap, ...historyKeymap, ...defaultKeymap]),
      // Markdown live-preview engine (mdlive). Placed AFTER the ⌘⏎/⌃⏎ chord
      // handler (Prec.highest, earlier in this array) so the send/draft chords
      // keep precedence; the engine's own Prec.highest Enter then drives tight-
      // list continuation on a PLAIN Enter. Markdown stays the literal value.
      ...livePreviewExtensions({ sourceMode }),
      ...(vimExt ? [vimExt] : []),
    ],
    [
      theme,
      sessionId,
      placeholder,
      vim,
      vimExt,
      aboveCursor,
      touchInput,
      sourceMode,
    ],
  );

  // Pixel-exact MUI `OutlinedInput` (no-label, size="small"), replicated rather
  // than wrapped: the editable is a CM `contenteditable`, not an <input>, so we
  // can't hand it to MUI's InputBase. Instead the chrome is an absolutely-
  // positioned `<fieldset>` (MUI's "notched outline" technique) using MUI's own
  // tokens — rest rgba(…,.23), hover text.primary, focus primary.main at 2px —
  // so the 1px→2px focus transition costs no reflow, identical to MUI.
  const restBorder = theme.palette.mode === "light"
    ? "rgba(0, 0, 0, 0.23)"
    : "rgba(255, 255, 255, 0.23)";
  return (
    <Box
      data-mobile-pager-ignore
      data-mobile-drawer-ignore
      onMouseDown={(e): void => {
        // Click anywhere in the padding focuses the editor, like a real input.
        if (e.target === e.currentTarget) {
          e.preventDefault();
          cmRef.current?.view?.focus();
        }
      }}
      sx={{
        position: "relative",
        borderRadius: `${theme.shape.borderRadius}px`,
        // Transparent like a real MUI OutlinedInput — inherits the composer
        // bar's surface so there's never a background mismatch.
        bgcolor: "transparent",
        // Fill mode (column layout): stretch to the flex parent's height and let
        // the inner CodeMirror own the scroll. `minHeight: 0` lets it shrink
        // below content inside the flex column so the scroller, not the page, grows.
        ...(fill && {
          flex: 1,
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          "& > *, & .cm-theme-none, & .cm-editor, & .cm-scroller": {
            flex: 1,
            minHeight: 0,
            height: "100%",
          },
        }),
        // Put the visual input padding ON the editable element. Keeping it on
        // this non-editable wrapper made the compact composer look like one
        // large text field while iOS could only long-press the 14px-high glyph
        // line; a hold in the surrounding blank area was treated as an outside
        // touch, dismissing the keyboard without showing Paste. With content
        // padding, the entire visible writing surface is a native selection
        // target and WebKit owns the long-press menu end to end.
        "& .cm-content": {
          boxSizing: "border-box",
          minHeight: fill ? "100%" : "24px",
          paddingTop: "8.5px",
          paddingBottom: "8.5px",
          paddingLeft: "14px",
          // Clear the overlaid expand/action controls at the right edge.
          paddingRight: `${String(14 + endInset)}px`,
        },
        // In the split Desktop composer the scrollbar remains flush-right;
        // padding belongs to content, so no wrapper compensation is needed.
        ...(flushRightScrollbar && { pr: 0 }),
        cursor: "text",
        "&:hover .composer-notch": disabled || borderless
          ? {}
          : { borderColor: "text.primary" },
        "&:focus-within .composer-notch": borderless ? {} : {
          borderColor: "primary.main",
          borderWidth: "2px",
        },
      }}
    >
      {!borderless && (
        <Box
          component="fieldset"
          aria-hidden="true"
          className="composer-notch"
          sx={{
            position: "absolute",
            inset: 0,
            m: 0,
            p: 0,
            pointerEvents: "none",
            borderRadius: "inherit",
            borderStyle: "solid",
            borderWidth: "1px",
            borderColor: disabled ? "action.disabled" : restBorder,
            transition: theme.transitions.create(
              ["border-color", "border-width"],
              { duration: theme.transitions.duration.shorter },
            ),
          }}
        />
      )}
      <CodeMirror
        ref={cmRef}
        value={ensureTrailingImageLine(value)}
        {...(initialSelection !== undefined
          ? {
            selection: {
              anchor: Math.min(
                Math.max(0, initialSelection),
                ensureTrailingImageLine(value).length,
              ),
            },
          }
          : {})}
        onChange={handleChange}
        editable={!disabled}
        autoFocus={autoFocus}
        // `none` disables @uiw's built-in light theme (which paints the editor
        // white); our cmTheme keeps it transparent so it inherits the lavender
        // composer surface — no white box.
        theme="none"
        basicSetup={false}
        extensions={extensions}
        // Fill: height:100% so the editor stretches to the (flex:1) wrapper above,
        // which itself fills the column — the `style` flex:1/minHeight:0 makes the
        // ReactCodeMirror wrapper div participate so .cm-editor's 100% resolves.
        {...(fill
          ? { height: "100%", minHeight: "0" }
          : {
            minHeight: expanded ? expandedHeight : "24px",
            maxHeight: expanded ? expandedHeight : "40vh",
          })}
        style={fill ? { flex: 1, minHeight: 0 } : undefined}
        indentWithTab={false}
      />
    </Box>
  );
});
