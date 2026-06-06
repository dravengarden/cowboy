import { forwardRef, useImperativeHandle, useMemo, useRef } from "react";
import { TextField } from "@mui/material";
import type { ComposerEditorHandle } from "./ComposerEditor";

// True on touch devices (coarse pointer). Computed once — a device doesn't flip
// pointer type mid-session, and the composer remounts per session anyway.
export function useTouchComposer(): boolean {
  return useMemo(
    () =>
      typeof window !== "undefined" &&
      window.matchMedia("(pointer: coarse)").matches,
    [],
  );
}

// Native-<textarea> composer for TOUCH devices, exposing the same
// ComposerEditorHandle as the CodeMirror one so callers swap them transparently.
//
// Why: CodeMirror's contenteditable desyncs IME composition on iOS Safari — the
// in-progress pinyin gets stranded at the line start while the committed
// characters land elsewhere. It's a documented CM6 / WebKit limitation (the
// editor maintains its own doc↔DOM position mapping that Safari breaks mid-
// composition) with no upstream fix on the latest @codemirror/view. A native
// textarea hands IME entirely to the OS, so Chinese/Japanese input is always
// correct. Accepted trade-offs on touch: the iOS keyboard accessory bar returns,
// and the inline @/​/ autocomplete is gone — the action-row buttons insert the
// trigger char (insertTrigger) and the user types the rest. Desktop keeps
// CodeMirror (ComposerEditor) for vim + live completion.
//
// Controlled (value/onChange): React suppresses onChange during an IME
// composition and only fires it on commit, so the controlled value never resets
// mid-composition — the contenteditable caret-bounce that forced CM uncontrolled
// does not apply to a native textarea.
export const ComposerTextarea = forwardRef<
  ComposerEditorHandle,
  {
    value: string;
    onChange: (value: string) => void;
    onSubmit: () => void;
    placeholder?: string;
    disabled?: boolean;
    onEscape?: () => boolean;
    onPasteFiles?: (files: File[]) => void;
  }
>(function ComposerTextarea(
  { value, onChange, onSubmit, placeholder, disabled, onEscape, onPasteFiles },
  ref,
): React.JSX.Element {
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useImperativeHandle(ref, () => ({
    focus: (): void => inputRef.current?.focus(),
    insertTrigger: (ch: string): void => {
      const ta = inputRef.current;
      const at = ta?.selectionStart ?? value.length;
      const to = ta?.selectionEnd ?? at;
      onChange(value.slice(0, at) + ch + value.slice(to));
      ta?.focus();
      // Caret lands just after the inserted char once the controlled value
      // re-renders the textarea.
      requestAnimationFrame(() => {
        const p = at + ch.length;
        inputRef.current?.setSelectionRange(p, p);
      });
    },
    // Controlled by `value`; the caller empties it via onChange("") on submit,
    // so there's nothing to clear imperatively.
    clear: (): void => undefined,
  }));

  return (
    <TextField
      inputRef={inputRef}
      value={value}
      onChange={(e): void => onChange(e.target.value)}
      onKeyDown={(e): void => {
        // Hardware keyboard (e.g. iPad): Cmd/Ctrl+Enter sends; plain Enter stays
        // a newline. Escape defers to the caller (cancel a running turn).
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
          e.preventDefault();
          onSubmit();
        } else if (e.key === "Escape" && onEscape?.()) {
          e.preventDefault();
        }
      }}
      onPaste={(e): void => {
        const files = Array.from(e.clipboardData.files);
        if (files.length > 0 && onPasteFiles) {
          e.preventDefault();
          onPasteFiles(files);
        }
      }}
      placeholder={placeholder}
      disabled={disabled}
      multiline
      minRows={1}
      maxRows={10}
      size="small"
      fullWidth
      sx={{
        // 16px floor so iOS never focus-zooms the input; 1rem so it tracks the
        // global font-size zoom (useGlobalFontScale). Matches cmTheme's sizing.
        "& .MuiInputBase-input": {
          fontSize: "max(16px, 1rem)",
          lineHeight: 1.5,
        },
      }}
    />
  );
});
