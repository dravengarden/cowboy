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
import { fileChipPlugin } from "./fileTokenWidget";
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
      fileChipPlugin,
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
      // completionKeymap first so Enter/Tab/arrows drive the picker when it's
      // open, falling through to newline/normal editing when it's closed.
      keymap.of([...completionKeymap, ...historyKeymap, ...defaultKeymap]),
      ...(vimExt ? [vimExt] : []),
    ],
    [theme, sessionId, placeholder, vimExt],
  );

  return (
    <Box
      sx={{
        border: 1,
        borderColor: disabled ? "action.disabledBackground" : "divider",
        borderRadius: 2,
        bgcolor: "background.paper",
        px: 1.5,
        py: 0.25,
        // Subtle, MUI-input-like: a 1px border that simply recolors on focus —
        // no heavy glow/double-border.
        transition: "border-color 120ms",
        "&:hover": disabled ? {} : { borderColor: "text.secondary" },
        "&:focus-within": { borderColor: "primary.main" },
      }}
    >
      <CodeMirror
        ref={cmRef}
        value={value}
        onChange={onChange}
        editable={!disabled}
        basicSetup={false}
        extensions={extensions}
        minHeight="24px"
        maxHeight="40vh"
        indentWithTab={false}
      />
    </Box>
  );
});
