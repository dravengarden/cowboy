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
import { EditorView, keymap, placeholder as placeholderExt } from "@codemirror/view";
import { Prec, type Extension } from "@codemirror/state";
import {
  autocompletion,
  completionKeymap,
  startCompletion,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { cmTheme } from "./cmTheme";
import { deleteTokenBackward, tokenChipPlugin } from "./fileTokenWidget";
import {
  fileCompletionSource,
  slashCompletionSource,
} from "./composerCompletions";
import type { AvailableCommand } from "./protocol";

export interface ComposerEditorHandle {
  focus: () => void;
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
  getCM: (view: EditorView) => { state?: { vim?: { insertMode?: boolean } } } | null;
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
    const desktop =
      typeof window !== "undefined" &&
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
  }
>(function ComposerEditor(
  { value, onChange, onSubmit, sessionId, commands, placeholder, disabled, vim, onEscape },
  ref,
): React.JSX.Element {
  const theme = useTheme();
  const cmRef = useRef<ReactCodeMirrorRef>(null);
  // Keep latest callbacks/data in refs so the (memoized) extensions never go
  // stale without rebuilding the editor state on every keystroke.
  const onSubmitRef = useRef(onSubmit);
  onSubmitRef.current = onSubmit;
  const commandsRef = useRef(commands);
  commandsRef.current = commands;
  const onEscapeRef = useRef(onEscape);
  onEscapeRef.current = onEscape;
  // Set by useVimExtension once the lazy vim module loads; read by the Escape
  // keymap to tell insert mode from normal/visual.
  const vimApiRef = useRef<VimApi | null>(null);

  const vimExt = useVimExtension(vim ?? false, vimApiRef);

  useImperativeHandle(ref, () => ({
    focus: (): void => cmRef.current?.view?.focus(),
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
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "" } });
    },
  }));

  const extensions = useMemo<Extension[]>(
    () => [
      EditorView.lineWrapping,
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
      }),
      // Cmd/Ctrl+Enter sends, at highest precedence so it beats vim's and the
      // default Enter binding. Plain Enter stays a newline.
      Prec.highest(
        keymap.of([
          {
            key: "Mod-Enter",
            run: (): boolean => {
              onSubmitRef.current();
              return true;
            },
          },
        ]),
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
              const insert = vimApiRef.current?.getCM(view)?.state?.vim?.insertMode ?? false;
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
    [theme, sessionId, placeholder, vimExt],
  );

  // Pixel-exact MUI `OutlinedInput` (no-label, size="small"), replicated rather
  // than wrapped: the editable is a CM `contenteditable`, not an <input>, so we
  // can't hand it to MUI's InputBase. Instead the chrome is an absolutely-
  // positioned `<fieldset>` (MUI's "notched outline" technique) using MUI's own
  // tokens — rest rgba(…,.23), hover text.primary, focus primary.main at 2px —
  // so the 1px→2px focus transition costs no reflow, identical to MUI.
  const restBorder =
    theme.palette.mode === "light"
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
