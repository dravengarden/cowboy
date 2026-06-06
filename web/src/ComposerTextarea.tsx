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
// Why a native textarea: CodeMirror's contenteditable desyncs IME composition on
// iOS Safari (pinyin strands at the line start) — a documented CM6/WebKit limit
// with no upstream fix. A native textarea hands IME to the OS, so CJK input is
// always correct.
//
// NB: an inline @/​/ picker was tried here and removed — its deferred
// (requestAnimationFrame) refocus put iOS into a phantom keyboard-focus state
// that `interactive-widget=resizes-content` shrank the layout viewport for
// without showing a keyboard, leaving a dead gap below the UI. On touch the
// action-row buttons just insert the trigger char and the user types the rest;
// the live picker stays desktop-only (CodeMirror).
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
    focusEnd: (): void => {
      const ta = inputRef.current;
      if (!ta) return;
      ta.focus();
      const end = ta.value.length;
      ta.setSelectionRange(end, end);
    },
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
        // Keep a usable tap target even when a small font scale shrinks the
        // text: the input box shouldn't collapse to a thin sliver. The textarea
        // still grows from here as you type (top-aligned).
        "& .MuiInputBase-root": {
          minHeight: 44,
          alignItems: "flex-start",
        },
        // `1rem` so the input tracks the reading font-size setting both up and
        // down (useGlobalFontScale scales the root font-size); line-height
        // follows the reading line-height var. No 16px floor — the installed PWA
        // disables focus-zoom via the viewport meta. Matches cmTheme's sizing.
        "& .MuiInputBase-input": {
          fontSize: "1rem",
          lineHeight: "var(--cowboy-reading-line-height, 1.5)",
        },
      }}
    />
  );
});
