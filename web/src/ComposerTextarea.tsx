import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { Box, List, ListItemButton, ListItemText, Paper, TextField } from "@mui/material";
import type { ComposerEditorHandle } from "./ComposerEditor";
import type { AvailableCommand } from "./protocol";

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

// One picker row.
interface PickerOption {
  /** Text to splice in (replaces the trigger token); includes a trailing space. */
  apply: string;
  primary: string;
  secondary?: string;
}

// The active `@`/`/` trigger token at the caret, if any.
interface Trigger {
  type: "@" | "/";
  /** Index of the trigger char in the value. */
  from: number;
  query: string;
}

// Find an active trigger token immediately before the caret. Mirrors the desktop
// CodeMirror rules (composerCompletions.ts): `/` only at the very start of the
// input; `@` at the start or after whitespace. Returns null when neither applies
// (caret moved away / a space ended the token / the trigger was deleted), which
// is what dismisses the popup.
function computeTrigger(value: string, caret: number): Trigger | null {
  const before = value.slice(0, caret);
  const slash = /^\/(\S*)$/u.exec(before);
  if (slash) return { type: "/", from: 0, query: slash[1] ?? "" };
  const at = /(?:^|\s)@(\S*)$/u.exec(before);
  if (at) {
    const q = at[1] ?? "";
    return { type: "@", from: caret - q.length - 1, query: q };
  }
  return null;
}

async function fetchFileOptions(sessionId: string, query: string): Promise<PickerOption[]> {
  const url = `/api/sessions/${encodeURIComponent(sessionId)}/files?q=${
    encodeURIComponent(query)
  }&limit=20`;
  try {
    const r = await fetch(url);
    if (!r.ok) return [];
    const d = (await r.json()) as { files?: string[] };
    return (d.files ?? []).map((path) => {
      const slash = path.lastIndexOf("/");
      return {
        apply: `@${path} `,
        primary: slash >= 0 ? path.slice(slash + 1) : path,
        ...(slash >= 0 ? { secondary: path.slice(0, slash) } : {}),
      };
    });
  } catch {
    return [];
  }
}

function slashOptions(commands: AvailableCommand[], query: string): PickerOption[] {
  const q = query.toLowerCase();
  return commands
    .filter((c) => c.name.toLowerCase().includes(q))
    .map((c) => ({ apply: `/${c.name} `, primary: `/${c.name}`, secondary: c.description }));
}

// Native-<textarea> composer for TOUCH devices, exposing the same
// ComposerEditorHandle as the CodeMirror one so callers swap them transparently.
//
// Why a native textarea: CodeMirror's contenteditable desyncs IME composition on
// iOS Safari (pinyin strands at the line start) — a documented CM6/WebKit limit
// with no upstream fix. A native textarea hands IME to the OS, so CJK input is
// always correct.
//
// The `@`/`/` picker (this file): since CM's inline autocomplete isn't available
// here, we render our own — a filtered list anchored ABOVE the input (the Slack /
// iMessage / Telegram idiom: stays in context above the keyboard, not a modal).
// `@` fuzzy-searches files via the daemon; `/` lists agent commands. Tapping a
// row splices the token in. Matches the desktop completion data + token format.
export const ComposerTextarea = forwardRef<
  ComposerEditorHandle,
  {
    value: string;
    onChange: (value: string) => void;
    onSubmit: () => void;
    sessionId: string;
    commands: () => AvailableCommand[];
    placeholder?: string;
    disabled?: boolean;
    onEscape?: () => boolean;
    onPasteFiles?: (files: File[]) => void;
  }
>(function ComposerTextarea(
  { value, onChange, onSubmit, sessionId, commands, placeholder, disabled, onEscape, onPasteFiles },
  ref,
): React.JSX.Element {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [trigger, setTrigger] = useState<Trigger | null>(null);
  const [options, setOptions] = useState<PickerOption[]>([]);
  const commandsRef = useRef(commands);
  commandsRef.current = commands;

  // Recompute the active trigger from the current value + caret. Called after any
  // edit, caret move, or selection change.
  const sync = (v: string, caret: number): void => setTrigger(computeTrigger(v, caret));

  // Resolve options for the active trigger: commands synchronously, files via a
  // debounced fetch (so each keystroke doesn't spam the endpoint).
  useEffect(() => {
    if (!trigger) {
      setOptions([]);
      return undefined;
    }
    if (trigger.type === "/") {
      setOptions(slashOptions(commandsRef.current(), trigger.query));
      return undefined;
    }
    let cancelled = false;
    const t = setTimeout(() => {
      void fetchFileOptions(sessionId, trigger.query).then((opts) => {
        if (!cancelled) setOptions(opts);
      });
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [trigger?.type, trigger?.query, sessionId]);

  const applyOption = (apply: string): void => {
    if (!trigger) return;
    const end = trigger.from + 1 + trigger.query.length; // trigger char + query
    const next = value.slice(0, trigger.from) + apply + value.slice(end);
    onChange(next);
    setTrigger(null);
    const pos = trigger.from + apply.length;
    requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.setSelectionRange(pos, pos);
    });
  };

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
      const next = value.slice(0, at) + ch + value.slice(to);
      onChange(next);
      ta?.focus();
      // Caret lands just after the inserted char once the controlled value
      // re-renders; then open the picker for that fresh trigger.
      const pos = at + ch.length;
      requestAnimationFrame(() => {
        inputRef.current?.setSelectionRange(pos, pos);
        sync(next, pos);
      });
    },
    clear: (): void => undefined,
  }));

  const popup = trigger && options.length > 0 && (
    <Paper
      elevation={6}
      sx={{
        position: "absolute",
        bottom: "100%",
        left: 0,
        right: 0,
        mb: 0.5,
        maxHeight: "40vh",
        overflowY: "auto",
        borderRadius: 1.5,
        // Float above any queue/draft panels stacked above the input.
        zIndex: 4,
      }}
    >
      <List dense disablePadding>
        {options.map((o) => (
          <ListItemButton
            key={o.apply}
            // pointerdown (not click) so the insert fires before the textarea
            // blurs / the keyboard does anything; preventDefault keeps focus.
            onPointerDown={(e): void => {
              e.preventDefault();
              applyOption(o.apply);
            }}
          >
            <ListItemText
              primary={o.primary}
              secondary={o.secondary}
              slotProps={{
                primary: { noWrap: true, variant: "body2" },
                secondary: { noWrap: true, variant: "caption" },
              }}
            />
          </ListItemButton>
        ))}
      </List>
    </Paper>
  );

  return (
    <Box sx={{ position: "relative" }}>
      {popup}
      <TextField
        inputRef={inputRef}
        value={value}
        onChange={(e): void => {
          onChange(e.target.value);
          sync(e.target.value, e.target.selectionStart ?? e.target.value.length);
        }}
        onSelect={(e): void => {
          const ta = e.target as HTMLTextAreaElement;
          sync(ta.value, ta.selectionStart ?? ta.value.length);
        }}
        onKeyDown={(e): void => {
          // Escape closes the picker first; otherwise defers to the caller.
          if (e.key === "Escape" && trigger) {
            e.preventDefault();
            setTrigger(null);
            return;
          }
          // Hardware keyboard (e.g. iPad): Cmd/Ctrl+Enter sends; plain Enter
          // stays a newline. Escape defers to the caller (cancel a running turn).
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
            e.preventDefault();
            onSubmit();
          } else if (e.key === "Escape" && onEscape?.()) {
            e.preventDefault();
          }
        }}
        onBlur={(): void => setTrigger(null)}
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
    </Box>
  );
});
