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
}

// Desktop-only Vim. Loads `@replit/codemirror-vim` lazily, and ONLY when the
// device has a precise pointer + hover (a real keyboard) — touch never imports
// it, so it costs the mobile bundle nothing. (Plan Step 11 / REQ-2.)
function useVimExtension(enabled: boolean): Extension | null {
  const [ext, setExt] = useState<Extension | null>(null);
  useEffect(() => {
    const desktop =
      typeof window !== "undefined" &&
      window.matchMedia("(pointer: fine) and (hover: hover)").matches;
    if (!enabled || !desktop) {
      setExt(null);
      return undefined;
    }
    let alive = true;
    void import("@replit/codemirror-vim")
      .then((m) => {
        if (alive) setExt(m.vim());
      })
      .catch(() => {
        /* vim is best-effort; ignore load failures */
      });
    return () => {
      alive = false;
    };
  }, [enabled]);
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
  }
>(function ComposerEditor(
  { value, onChange, onSubmit, sessionId, commands, placeholder, disabled, vim },
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

  const vimExt = useVimExtension(vim ?? false);

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
