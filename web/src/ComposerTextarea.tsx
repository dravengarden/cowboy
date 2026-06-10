import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Box, Paper, TextField, Typography } from "@mui/material";
import type { ComposerEditorHandle } from "./ComposerEditor";
import { hasDraftMod, hasSendMod } from "./platform";
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

interface PickerOption {
  /** Text to splice in (replaces the trigger token); includes a trailing space. */
  apply: string;
  primary: string;
  secondary?: string;
}

// The active `@`/`/` trigger token at the caret, if any.
interface Trigger {
  type: "@" | "/";
  from: number;
  query: string;
}

// Find an active trigger token immediately before the caret — mirrors the desktop
// CodeMirror rules (composerCompletions.ts): `/` only at the very start of the
// input; `@` at the start or after whitespace. Returns null when neither applies
// (caret moved / a space ended the token / the trigger was deleted), which
// dismisses the popup.
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

async function fetchFileOptions(
  sessionId: string,
  query: string,
): Promise<PickerOption[]> {
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

function slashOptions(
  commands: AvailableCommand[],
  query: string,
): PickerOption[] {
  const q = query.toLowerCase();
  return commands
    .filter((c) => c.name.toLowerCase().includes(q))
    .map((c) => ({
      apply: `/${c.name} `,
      primary: `/${c.name}`,
      secondary: c.description,
    }));
}

// Native-<textarea> composer for TOUCH devices, exposing the same
// ComposerEditorHandle as the CodeMirror one so callers swap them transparently.
//
// Why a native textarea: CodeMirror's contenteditable desyncs IME composition on
// iOS Safari (pinyin strands at the line start) — a documented CM6/WebKit limit
// with no upstream fix. A native textarea hands IME to the OS, so CJK input is
// always correct.
//
// The `@`/`/` picker (this file): a filtered list anchored ABOVE the input (Slack
// / iMessage idiom — in context, above the keyboard, not a modal). CRITICAL: the
// option rows are NON-FOCUSABLE plain elements with `preventDefault` on
// mousedown, so tapping one NEVER blurs the textarea — the keyboard stays up
// through the whole pick and there's no deferred refocus (a focusable button +
// rAF refocus previously caused an iOS phantom-keyboard viewport gap; see
// [[ios-file-picker-keyboard-limit]] family). `@` fuzzy-searches files via the
// daemon; `/` lists agent commands. Matches the desktop completion data + token
// format.
export const ComposerTextarea = forwardRef<
  ComposerEditorHandle,
  {
    value: string;
    onChange: (value: string) => void;
    onSubmit: () => void;
    // ⌃⏎ (mac) / Alt+⏎ — park as a draft. Optional: the queued-message edit box
    // reuses this textarea and has no draft action, so it omits this.
    onSaveDraft?: () => void;
    sessionId: string;
    commands: () => AvailableCommand[];
    placeholder?: string;
    disabled?: boolean;
    onEscape?: () => boolean;
    onPasteFiles?: (files: File[]) => void;
    /// Right padding (px) reserved on the text so it never runs under the action
    /// buttons the composer overlays at the input's bottom-right. 0 when none.
    endInset?: number;
    /// Drop the TextField's outlined border so the surrounding composer Paper card
    /// owns the box (no double border). Mirrors ComposerEditor.borderless.
    borderless?: boolean;
    /// Tall editing area (the mobile fullscreen compose / edit sheet): grows from
    /// many rows and caps high so it fills the sheet instead of the compact
    /// auto-grow. Mirrors ComposerEditor.expanded.
    expanded?: boolean;
  }
>(function ComposerTextarea(
  {
    value,
    onChange,
    onSubmit,
    onSaveDraft,
    sessionId,
    commands,
    placeholder,
    disabled,
    onEscape,
    onPasteFiles,
    endInset = 0,
    borderless = false,
    expanded = false,
  },
  ref,
): React.JSX.Element {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [trigger, setTrigger] = useState<Trigger | null>(null);
  const [options, setOptions] = useState<PickerOption[]>([]);
  // Top edge (viewport px) of the input the picker is anchored to. The picker
  // opens UPWARD (bottom:100%), so the space above the input is its ceiling —
  // and that space SHRINKS when the keyboard is up. We cap the popup to it (see
  // the maxHeight calc) so its top can never overflow off-screen, clipping the
  // first options with no way to reach them (the reported bug). Remeasured while
  // the picker is open: on each keystroke (a multiline textarea grows, moving
  // its top) and on visualViewport resize (keyboard open/close, rotation).
  const [anchorTop, setAnchorTop] = useState(0);
  const pickerOpen = Boolean(trigger) && options.length > 0;
  useLayoutEffect(() => {
    if (!pickerOpen) return undefined;
    const measure = (): void => {
      const r = inputRef.current?.getBoundingClientRect();
      if (r) setAnchorTop(r.top);
    };
    measure();
    const vv = globalThis.visualViewport;
    vv?.addEventListener("resize", measure);
    vv?.addEventListener("scroll", measure);
    return () => {
      vv?.removeEventListener("resize", measure);
      vv?.removeEventListener("scroll", measure);
    };
  }, [pickerOpen, value]);
  const commandsRef = useRef(commands);
  commandsRef.current = commands;

  const sync = (v: string, caret: number): void =>
    setTrigger(computeTrigger(v, caret));

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

  // Splice the chosen token in. NO .focus() here: the option row's mousedown
  // preventDefault kept the textarea focused (keyboard never dropped), so we only
  // move the caret (in a rAF, after the controlled value re-renders). Calling
  // focus() — especially deferred — is what caused the iOS phantom-keyboard gap.
  const applyOption = (apply: string): void => {
    if (!trigger) return;
    const end = trigger.from + 1 + trigger.query.length;
    const next = value.slice(0, trigger.from) + apply + value.slice(end);
    onChange(next);
    setTrigger(null);
    const pos = trigger.from + apply.length;
    requestAnimationFrame(() => inputRef.current?.setSelectionRange(pos, pos));
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
      ta?.focus(); // synchronous, inside the toolbar-button tap gesture — safe
      const pos = at + ch.length;
      requestAnimationFrame(() => {
        inputRef.current?.setSelectionRange(pos, pos);
        sync(next, pos);
      });
    },
    clear: (): void => undefined,
    // Markdown toolbar actions are CM6-only (the fullscreen toolbar always mounts
    // ComposerEditor, never this textarea). No-ops here just satisfy the shared
    // ComposerEditorHandle — this component is retained solely as the documented
    // iOS-IME fallback for the collapsed mobile input (see plan Risks).
    wrap: (): void => undefined,
    toggleLinePrefix: (): void => undefined,
    cycleHeading: (): void => undefined,
    insertLink: (): void => undefined,
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
        // Cap to the space ABOVE the input so the popup never overflows the top
        // of the screen and clips its first rows. `anchorTop` is the input's
        // distance from the (visual) viewport top; minus the safe-area inset +
        // a gap is exactly the room available. clamp keeps a usable floor and
        // never exceeds 40vh on a tall/keyboard-less screen. overflowY:auto then
        // makes every option reachable by scrolling within that bound.
        maxHeight: anchorTop > 0
          ? `clamp(120px, calc(${
            String(anchorTop)
          }px - env(safe-area-inset-top, 0px) - 12px), 40vh)`
          : "40vh",
        overflowY: "auto",
        borderRadius: 1.5,
        zIndex: 4,
        py: 0.5,
      }}
    >
      {options.map((o) => (
        <Box
          key={o.apply}
          role="option"
          // preventDefault on mousedown keeps the textarea focused (the keyboard
          // never drops); a plain Box isn't focusable so it can't steal focus.
          // Select on click so the list can still be scrolled by dragging.
          onMouseDown={(e): void => e.preventDefault()}
          onClick={(): void => applyOption(o.apply)}
          sx={{
            px: 1.5,
            py: 0.625,
            cursor: "pointer",
            minWidth: 0,
            // Single line: bold-ish name + muted, ellipsized description on the
            // SAME row, so ~twice as many options fit in the cramped space above
            // the keyboard (command-palette density). Baseline-aligned so the
            // smaller description sits on the name's text baseline.
            display: "flex",
            alignItems: "baseline",
            gap: 1,
            "&:active": { bgcolor: "action.selected" },
            "@media (hover: hover)": { "&:hover": { bgcolor: "action.hover" } },
          }}
        >
          <Typography
            variant="body2"
            noWrap
            sx={{ fontWeight: 500, flexShrink: 0, maxWidth: "60%" }}
          >
            {o.primary}
          </Typography>
          {o.secondary != null && o.secondary !== "" && (
            <Typography
              variant="caption"
              color="text.secondary"
              noWrap
              sx={{ flex: 1, minWidth: 0 }}
            >
              {o.secondary}
            </Typography>
          )}
        </Box>
      ))}
    </Paper>
  );

  return (
    <Box
      sx={{
        position: "relative",
        // `expanded` (fullscreen compose / edit sheet): fill the sheet so the
        // textarea is a full-height writing canvas, not a fixed block with blank
        // space below it.
        ...(expanded && { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }),
      }}
    >
      {popup}
      <TextField
        inputRef={inputRef}
        value={value}
        onChange={(e): void => {
          onChange(e.target.value);
          sync(
            e.target.value,
            e.target.selectionStart ?? e.target.value.length,
          );
        }}
        onSelect={(e): void => {
          const ta = e.target as HTMLTextAreaElement;
          sync(ta.value, ta.selectionStart ?? ta.value.length);
        }}
        onKeyDown={(e): void => {
          if (e.key === "Escape" && trigger) {
            e.preventDefault();
            setTrigger(null);
            return;
          }
          if (e.key === "Enter" && hasDraftMod(e) && onSaveDraft) {
            // ⌃⏎ / Alt+⏎ → draft (e.g. an iPad with an external keyboard).
            e.preventDefault();
            onSaveDraft();
          } else if (e.key === "Enter" && hasSendMod(e)) {
            // ⌘⏎ only — Ctrl+Enter no longer sends (it's the draft chord now).
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
        minRows={expanded ? 10 : 1}
        maxRows={expanded ? 30 : 10}
        size="small"
        fullWidth
        sx={{
          // `expanded` fills the sheet; the textarea is forced to 100% height (the
          // !important overrides MUI's autosize inline height) and scrolls within.
          ...(expanded && { flex: 1, minHeight: 0, display: "flex" }),
          // Keep a usable tap target even when a small font scale shrinks the
          // text. The textarea still grows from here as you type (top-aligned).
          "& .MuiInputBase-root": {
            minHeight: 44,
            alignItems: expanded ? "stretch" : "flex-start",
            ...(expanded && { height: "100%" }),
          },
          // `1rem` so the input tracks the reading font-size setting both up and
          // down; line-height follows the reading line-height var. No 16px floor
          // — the installed PWA disables focus-zoom via the viewport meta.
          "& .MuiInputBase-input": {
            fontSize: "1rem",
            lineHeight: "var(--cowboy-reading-line-height, 1.5)",
            // Clear the overlaid send/kebab buttons at the bottom-right.
            ...(endInset > 0 && { paddingRight: `${String(endInset)}px` }),
            ...(expanded && { height: "100% !important", overflowY: "auto !important" }),
          },
          // Inside the composer's outlined Paper card the card draws the box, so
          // drop the TextField's own outline in all three states (rest/hover/focus).
          ...(borderless && {
            "& .MuiOutlinedInput-notchedOutline": { border: "none" },
            "&:hover .MuiOutlinedInput-notchedOutline": { border: "none" },
            "& .Mui-focused .MuiOutlinedInput-notchedOutline": { border: "none" },
          }),
        }}
      />
    </Box>
  );
});
