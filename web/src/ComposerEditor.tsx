import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
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
  autocompletion,
  completionKeymap,
  startCompletion,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { cmTheme } from "./cmTheme";
import { hasDraftMod, hasSendMod } from "./platform";
import { deleteTokenBackward, tokenChipPlugin } from "./fileTokenWidget";
import {
  fileCompletionSource,
  slashCompletionSource,
} from "./composerCompletions";
import type { AvailableCommand } from "./protocol";

export interface ComposerEditorHandle {
  focus: () => void;
  // Focus AND place the caret at the very end of the document — used when
  // opening an existing draft/queued message for editing, so you continue from
  // where the text left off instead of with the caret stranded at the start.
  focusEnd: () => void;
  // Insert a trigger char (`/` or `@`) at the caret + open the picker — used by
  // the action-row buttons. Mirrors the old `appendToken` + focus behavior.
  insertTrigger: (ch: string) => void;
  // Clear the document imperatively. Submit can't rely on the controlled
  // `value=""` prop to empty the editor: @uiw/react-codemirror (≥4.24) holds a
  // 200ms "typing latch" and DEFERS external value-prop changes while you're
  // still within 200ms of your last keystroke (an IME-echo guard). Hitting
  // Cmd-Enter right after typing lands inside that window, so the prop-driven
  // clear gets parked until the latch expires — the text lingers after the
  // message already sent. Dispatching straight to the view bypasses the latch
  // and clears now.
  clear: () => void;
}

// Reads whether the editor is in Vim *insert* mode, via the loaded vim module's
// CM5-compat handle. Lets the Escape keymap (below) decide whether Esc should
// exit insert mode (vim's job) or bubble up to the app (cancel a running turn).
type VimApi = {
  getCM: (
    view: EditorView,
  ) => { state?: { vim?: { insertMode?: boolean } } } | null;
};

// Desktop-only Vim. Loads `@replit/codemirror-vim` lazily, and ONLY when the
// device has a precise pointer + hover (a real keyboard) — touch never imports
// it, so it costs the mobile bundle nothing. (Plan Step 11 / REQ-2.) Also
// publishes the module's `getCM` into `apiRef` so the Escape handler can read
// the live vim mode without re-importing.
function useVimExtension(
  enabled: boolean,
  apiRef: { current: VimApi | null },
): Extension | null {
  const [ext, setExt] = useState<Extension | null>(null);
  useEffect(() => {
    const desktop = typeof window !== "undefined" &&
      window.matchMedia("(pointer: fine) and (hover: hover)").matches;
    if (!enabled || !desktop) {
      setExt(null);
      apiRef.current = null;
      return undefined;
    }
    let alive = true;
    void import("@replit/codemirror-vim")
      .then((m) => {
        if (alive) {
          setExt(m.vim());
          apiRef.current = { getCM: m.getCM as VimApi["getCM"] };
        }
      })
      .catch(() => {
        /* vim is best-effort; ignore load failures */
      });
    return () => {
      alive = false;
      apiRef.current = null;
    };
  }, [enabled, apiRef]);
  return ext;
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
    vim?: boolean;
    // Called on Escape when it should act on the app, not the editor: vim OFF,
    // or vim ON and already in normal/visual mode. Returns true if it consumed
    // the key. In vim insert mode Esc is left to the vim extension (→ normal),
    // so the first Esc exits insert and the second reaches here (Zed-style).
    onEscape?: () => boolean;
    // Called with image / file blobs found on a clipboard paste (a screenshot,
    // a copied image). When it handles them the editor swallows the paste so
    // no stray base64 / filename text lands in the document.
    onPasteFiles?: (files: File[]) => void;
    /// Right padding (px) reserved on the editor content so text never runs under
    /// the action buttons the composer overlays at the input's bottom-right.
    endInset?: number;
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
    vim,
    onEscape,
    onPasteFiles,
    endInset = 0,
  },
  ref,
): React.JSX.Element {
  const theme = useTheme();
  const cmRef = useRef<ReactCodeMirrorRef>(null);
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
  // Set by useVimExtension once the lazy vim module loads; read by the Escape
  // keymap to tell insert mode from normal/visual.
  const vimApiRef = useRef<VimApi | null>(null);

  const vimExt = useVimExtension(vim ?? false, vimApiRef);

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

  useImperativeHandle(ref, () => ({
    focus: (): void => cmRef.current?.view?.focus(),
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
    clear: (): void => {
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
  }));

  const extensions = useMemo<Extension[]>(
    () => [
      EditorView.lineWrapping,
      // Clipboard paste of image / file blobs (a screenshot, a copied image)
      // is lifted out to the composer as attachments; only a files-bearing
      // paste is swallowed, so plain-text paste keeps CodeMirror's behaviour.
      EditorView.domEventHandlers({
        paste: (event): boolean => {
          const cb = event.clipboardData;
          if (!cb) return false;
          const files = Array.from(cb.files);
          if (files.length === 0 || !onPasteFilesRef.current) return false;
          event.preventDefault();
          onPasteFilesRef.current(files);
          return true;
        },
        // iOS IME fix. The `.cm-scroller` compositing layer (translateZ(0) in
        // cmTheme — there to force WebKit to repaint typed text inside the
        // position:fixed body) corrupts IME marked-text rendering on iOS Safari:
        // mid-composition the pinyin paints at the line start, IN FRONT of the
        // already-committed characters (WebKit mis-places the composition overlay
        // relative to the promoted layer). Drop the layer for the duration of the
        // composition — the IME paints its own marked text while composing, so the
        // repaint hack isn't needed then — and restore it on commit, with a
        // one-frame opacity nudge so the just-committed glyphs repaint (the same
        // trick clear() uses). Composition events aren't `key*`, so CM's
        // ignoreDuringComposition lets them through to these handlers. Returning
        // false leaves CM's own composition handling untouched.
        compositionstart: (_event, view): boolean => {
          view.scrollDOM.style.transform = "none";
          return false;
        },
        compositionend: (_event, view): boolean => {
          view.scrollDOM.style.transform = "";
          const content = view.contentDOM;
          content.style.opacity = "0.999";
          requestAnimationFrame(() => {
            content.style.opacity = "";
          });
          return false;
        },
        // Self-heal the compositing layer on focus loss. Safari can interrupt a
        // composition WITHOUT firing compositionend — most notably the native
        // photo picker (the attach button) stealing focus mid-pinyin. That would
        // strand the scroller at `transform: none` (set on compositionstart),
        // reviving the very "typed text won't repaint" bug the layer exists to
        // fix — so after attaching an image, the next keystrokes misbehave.
        // A blur always restores the layer, so whatever state a half-finished
        // composition left, losing focus puts it back.
        blur: (_event, view): boolean => {
          view.scrollDOM.style.transform = "";
          return false;
        },
      }),
      history(),
      placeholderExt(placeholder ?? ""),
      cmTheme(theme),
      tokenChipPlugin,
      autocompletion({
        override: [
          fileCompletionSource(sessionId),
          slashCompletionSource(() => commandsRef.current()),
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
          keydown: (e): boolean => {
            if (e.key !== "Enter" || e.shiftKey || e.isComposing) return false;
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
      // Backspace removes a whole `@path` / `/skill` chip in one press (above
      // the default char-delete).
      Prec.high(keymap.of([{ key: "Backspace", run: deleteTokenBackward }])),
      // Escape: in vim insert mode, yield (return false) so the vim extension
      // takes it as exit-to-normal — the SECOND Esc, now in normal mode, reaches
      // onEscape. With vim off, or already in normal/visual, Esc goes straight to
      // onEscape (the app uses it to cancel a running turn). High precedence so
      // it beats the default keymap's Escape (clear-selection).
      Prec.high(
        keymap.of([
          {
            key: "Escape",
            run: (view): boolean => {
              const insert =
                vimApiRef.current?.getCM(view)?.state?.vim?.insertMode ?? false;
              if (insert) return false;
              return onEscapeRef.current?.() ?? false;
            },
          },
        ]),
      ),
      // completionKeymap first so Enter/Tab/arrows drive the picker when it's
      // open, falling through to newline/normal editing when it's closed.
      keymap.of([...completionKeymap, ...historyKeymap, ...defaultKeymap]),
      ...(vimExt ? [vimExt] : []),
    ],
    [theme, sessionId, placeholder, vimExt, aboveCursor],
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
        // OutlinedInput small content padding.
        px: "14px",
        py: "8.5px",
        // Clear the overlaid send/kebab buttons at the bottom-right (base 14 + inset).
        ...(endInset > 0 && { pr: `${String(14 + endInset)}px` }),
        cursor: "text",
        "&:hover .composer-notch": disabled
          ? {}
          : { borderColor: "text.primary" },
        "&:focus-within .composer-notch": {
          borderColor: "primary.main",
          borderWidth: "2px",
        },
      }}
    >
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
      <CodeMirror
        ref={cmRef}
        value={value}
        onChange={onChange}
        editable={!disabled}
        // `none` disables @uiw's built-in light theme (which paints the editor
        // white); our cmTheme keeps it transparent so it inherits the lavender
        // composer surface — no white box.
        theme="none"
        basicSetup={false}
        extensions={extensions}
        minHeight="24px"
        maxHeight="40vh"
        indentWithTab={false}
      />
    </Box>
  );
});
