import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Box, Paper, Typography } from "@mui/material";
import type {
  ComposerEditorHandle,
  ComposerEditorSelection,
} from "./ComposerEditor";
import { type Attachment, clipboardFiles } from "./attachments";
import { readWebClipboard } from "./composer/webClipboard";
import { insertNativeInlineImages } from "./composer/mobileCompactEditorPolicy";
import { attachComposerInputDebug } from "./composer/composerInputDebug";
import { reportMobileNativePasteEvent } from "./composer/mobileNativePasteTelemetry";
import { hasDraftMod, hasSendMod } from "./platform";
import { isImeKeyEvent } from "./imeKey";
import type { AvailableCommand } from "./protocol";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import {
  cycleNativeHeading,
  indentNativeLines,
  insertNativeCodeBlock,
  insertNativeLink,
  mapNativeSelectionThroughValueChange,
  nativeTextareaFittedHeight,
  nativeTextareaNeedsScroll,
  type NativeTextEdit,
  outdentNativeLines,
  replaceNativeSelection,
  setNativeHeading,
  toggleNativeCheckbox,
  toggleNativeLinePrefix,
  toggleNativeWrap,
  wrapNativeSelection,
} from "./composer/nativeTextareaEditing";
// Compatibility hook for composer call sites. Platform classification is owned
// centrally by SurfaceProvider so every part of the app agrees on the active
// interaction model (especially iPad + trackpad and hybrid devices).
export function useTouchComposer(): boolean {
  return useSurfaceProfile().kind !== "desktop";
}

interface PickerOption {
  /** Text to splice in (replaces the trigger token); includes a trailing space. */
  apply: string;
  primary: string;
  secondary?: string;
  slashCommand?: string;
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
      slashCommand: c.name,
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
    autoFocus?: boolean;
    /** One-shot CM6 -> native selection restored during the replacement commit. */
    initialSelection?: ComposerEditorSelection;
    onEscape?: () => boolean;
    onPasteFiles?: (files: File[]) => void;
    /** Synchronous native -> CM6 handoff point for an inline-image insert. */
    onInlineImageInsertion?: (caret: number, preserveFocus: boolean) => void;
    onSelectionChange?: (hasSelection: boolean) => void;
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
    autoFocus = false,
    initialSelection,
    onEscape,
    onPasteFiles,
    onInlineImageInsertion,
    onSelectionChange,
    endInset = 0,
    borderless = false,
    expanded = false,
  },
  ref,
): React.JSX.Element {
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const initialSelectionRef = useRef(initialSelection);
  const lastSelectionRef = useRef<ComposerEditorSelection>(
    initialSelection ?? { anchor: value.length, head: value.length },
  );
  const onSelectionChangeRef = useRef(onSelectionChange);
  onSelectionChangeRef.current = onSelectionChange;
  // Keep the live textarea value out of React's controlled reconciliation. On
  // iOS, a long-press changes the native selection first and `onSelect` may
  // re-render the parent before UIKit presents Paste/Select. Rebinding `value`
  // during that render can replace the native selection and dismiss the menu.
  // This ref also protects a just-accepted keystroke while its React mirror is
  // one render behind.
  const lastNativeValueRef = useRef<string | null>(null);
  const selectedSlashCommandRef = useRef<string | null>(null);
  const [trigger, setTrigger] = useState<Trigger | null>(null);
  const [options, setOptions] = useState<PickerOption[]>([]);
  // A fitted MUI textarea commonly reports scrollHeight one CSS pixel taller
  // than clientHeight due to line-height rounding. Promoting that residue to an
  // iOS scroll container makes UIKit keep a stale caret rect after Return. Only
  // enable native scrolling for meaningful overflow; this state changes at the
  // fit/overflow boundary, not on every keystroke.
  const [nativeScrollable, setNativeScrollable] = useState(false);
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

  const fitNativeTextareaExactly = (ta: HTMLTextAreaElement): void => {
    // Read the natural content height only after releasing the previous inline
    // height. Reading scrollHeight first can return the stale tall box after a
    // long prompt was cleared while focused.
    ta.style.height = "auto";
    const needed = nativeTextareaFittedHeight(ta.scrollHeight);
    ta.style.height = `${String(needed)}px`;
  };

  const syncNativeScrollable = (ta: HTMLTextAreaElement): void => {
    const next = nativeTextareaNeedsScroll(ta.scrollHeight, ta.clientHeight);
    setNativeScrollable((current) => current === next ? current : next);
  };

  const measureNativeOverflow = (): void => {
    const ta = inputRef.current;
    if (!ta) return;
    if (!expanded) {
      const needed = nativeTextareaFittedHeight(ta.scrollHeight);
      // Collapsing to height:auto while focused races UIKit's caret overlay
      // after Return and leaves it painted on the previous line (pitfall #63).
      if (ta.ownerDocument.activeElement === ta) {
        const current = Number.parseFloat(ta.style.height) || ta.clientHeight;
        if (needed > current + 1) ta.style.height = `${String(needed)}px`;
      } else {
        fitNativeTextareaExactly(ta);
      }
    } else {
      ta.style.removeProperty("height");
    }
    syncNativeScrollable(ta);
  };

  useLayoutEffect(() => {
    measureNativeOverflow();
  }, [value, expanded]);

  useEffect(() => {
    const ta = inputRef.current;
    if (!ta || typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measureNativeOverflow);
    observer.observe(ta);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const ta = inputRef.current;
    if (!ta) return undefined;
    return attachComposerInputDebug(ta);
  }, []);

  const rememberSelection = (
    ta: HTMLTextAreaElement,
  ): ComposerEditorSelection => {
    const start = ta.selectionStart ?? 0;
    const end = ta.selectionEnd ?? start;
    const selection = ta.selectionDirection === "backward"
      ? { anchor: end, head: start }
      : { anchor: start, head: end };
    lastSelectionRef.current = selection;
    return selection;
  };

  const rememberedSelection = (
    ta: HTMLTextAreaElement,
  ): ComposerEditorSelection => {
    const clamp = (position: number): number =>
      Math.max(0, Math.min(position, ta.value.length));
    return {
      anchor: clamp(lastSelectionRef.current.anchor),
      head: clamp(lastSelectionRef.current.head),
    };
  };

  const publishSelection = (ta: HTMLTextAreaElement): void => {
    const selection = rememberSelection(ta);
    onSelectionChangeRef.current?.(selection.anchor !== selection.head);
  };

  // Deleting the last inline-image token replaces the focused CM6 editor with
  // this textarea. React's autoFocus transfers the existing UIKit keyboard in
  // the same commit; restore the logical selection without issuing a second
  // focus write. A timer/rAF here runs after the first-responder transaction and
  // lets the software keyboard collapse.
  useLayoutEffect(() => {
    const ta = inputRef.current;
    const selection = initialSelectionRef.current;
    if (!ta || selection === undefined) return;
    const clamp = (position: number): number =>
      Math.max(0, Math.min(position, ta.value.length));
    const anchor = clamp(selection.anchor);
    const head = clamp(selection.head);
    ta.setSelectionRange(
      Math.min(anchor, head),
      Math.max(anchor, head),
      anchor > head ? "backward" : "forward",
    );
    lastSelectionRef.current = { anchor, head };
    onSelectionChangeRef.current?.(anchor !== head);
  }, []);

  const currentTextSelection = (): {
    value: string;
    from: number;
    to: number;
  } => {
    const ta = inputRef.current;
    const current = ta?.value ?? value;
    const selection = ta
      ? (ta.ownerDocument.activeElement === ta
        ? rememberSelection(ta)
        : rememberedSelection(ta))
      : { anchor: current.length, head: current.length };
    const from = Math.min(selection.anchor, selection.head);
    const to = Math.max(selection.anchor, selection.head);
    return { value: current, from, to };
  };

  // React mirrors the draft for submit/persistence, but must not overwrite a
  // native value that was already accepted by the textarea. External changes
  // (clear, loading another draft, or a non-editor state transition) still get
  // applied here, with the old caret preserved as far as the new value allows.
  useLayoutEffect(() => {
    const ta = inputRef.current;
    if (!ta || ta.value === value) {
      if (ta?.value === value) lastNativeValueRef.current = null;
      return;
    }
    if (lastNativeValueRef.current === ta.value) return;
    const previous = ta.value;
    const from = ta.selectionStart ?? previous.length;
    const to = ta.selectionEnd ?? from;
    const selection = mapNativeSelectionThroughValueChange(
      previous,
      value,
      from,
      to,
    );
    ta.value = value;
    ta.setSelectionRange(selection.from, selection.to);
    rememberSelection(ta);
    lastNativeValueRef.current = null;
  }, [value]);

  const writeNativeEdit = (
    ta: HTMLTextAreaElement,
    edit: NativeTextEdit,
  ): void => {
    ta.value = edit.value;
    ta.setSelectionRange(edit.from, edit.to);
    rememberSelection(ta);
    lastNativeValueRef.current = edit.value;
  };

  // Accessory buttons prevent pointer-down default, so the native textarea is
  // still UIKit's first responder when this runs. Commit literal Markdown and
  // its selection synchronously; a delayed selection write after React paints
  // is enough to reset an iPad keyboard/selection transaction.
  const applyTextEdit = (edit: NativeTextEdit): void => {
    const ta = inputRef.current;
    ta?.focus();
    if (ta) writeNativeEdit(ta, edit);
    onChange(edit.value);
    setTrigger(null);
    if (ta) publishSelection(ta);
  };

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

  // Splice the chosen token in. The option row's mousedown preventDefault keeps
  // the textarea focused, so move the live native caret synchronously and do not
  // schedule a post-render selection write.
  const applyOption = (option: PickerOption): void => {
    if (!trigger) return;
    const { apply } = option;
    const current = currentTextSelection();
    const end = trigger.from + 1 + trigger.query.length;
    const next = current.value.slice(0, trigger.from) + apply +
      current.value.slice(end);
    const pos = trigger.from + apply.length;
    const ta = inputRef.current;
    if (ta) writeNativeEdit(ta, { value: next, from: pos, to: pos });
    onChange(next);
    selectedSlashCommandRef.current = option.slashCommand ?? null;
    setTrigger(null);
    if (ta) publishSelection(ta);
  };

  useImperativeHandle(ref, () => ({
    focus: (): void => inputRef.current?.focus(),
    hasFocus: (): boolean => {
      const ta = inputRef.current;
      return ta !== null && ta.ownerDocument.activeElement === ta;
    },
    // The browser owns selection reveal for the native textarea. Keep this a
    // no-op so a viewport resize never rewrites UIKit selection or IME state.
    revealSelection: (): void => undefined,
    getValue: (): string => inputRef.current?.value ?? value,
    getSelection: (): ComposerEditorSelection => {
      const ta = inputRef.current;
      if (!ta) return lastSelectionRef.current;
      return ta.ownerDocument.activeElement === ta
        ? rememberSelection(ta)
        : rememberedSelection(ta);
    },
    focusSelection: (selection: ComposerEditorSelection): void => {
      const ta = inputRef.current;
      if (!ta) return;
      const clamp = (position: number): number =>
        Math.max(0, Math.min(position, ta.value.length));
      const anchor = clamp(selection.anchor);
      const head = clamp(selection.head);
      ta.focus();
      ta.setSelectionRange(
        Math.min(anchor, head),
        Math.max(anchor, head),
        anchor > head ? "backward" : "forward",
      );
      lastSelectionRef.current = { anchor, head };
      publishSelection(ta);
    },
    // The native touch textarea has no Vim state; Escape belongs to its host.
    escapeBelongsToApp: (): boolean => true,
    focusEnd: (): void => {
      const ta = inputRef.current;
      if (!ta) return;
      ta.focus();
      const end = ta.value.length;
      ta.setSelectionRange(end, end);
      rememberSelection(ta);
    },
    insertTrigger: (ch: string): void => {
      const ta = inputRef.current;
      const current = currentTextSelection();
      const at = current.from;
      const next = current.value.slice(0, at) + ch +
        current.value.slice(current.to);
      const pos = at + ch.length;
      ta?.focus(); // synchronous, inside the toolbar-button tap gesture — safe
      if (ta) writeNativeEdit(ta, { value: next, from: pos, to: pos });
      onChange(next);
      sync(next, pos);
      if (ta) publishSelection(ta);
    },
    insertText: (
      text: string,
      capturedSelection?: ComposerEditorSelection,
    ): void => {
      if (text.length === 0) return;
      const ta = inputRef.current;
      const current = ta?.value ?? value;
      const anchor = capturedSelection?.anchor ??
        ta?.selectionStart ?? current.length;
      const head = capturedSelection?.head ??
        ta?.selectionEnd ?? anchor;
      applyTextEdit(replaceNativeSelection(current, anchor, head, text));
    },
    clear: (): void => {
      selectedSlashCommandRef.current = null;
      applyTextEdit({ value: "", from: 0, to: 0 });
    },
    consumeSelectedSlashCommand: (): string | null => {
      const command = selectedSlashCommandRef.current;
      selectedSlashCommandRef.current = null;
      return command;
    },
    // Preserve the placement token even though a native textarea cannot render
    // the widget itself. PlatformComposerEditor sees the controlled value gain
    // the token and immediately promotes this same document to CM6, where the
    // registered attachment renders inline at this exact position.
    insertImage: (attachment: Attachment): void => {
      const ta = inputRef.current;
      const current = ta?.value ?? value;
      const at = ta?.selectionStart ?? current.length;
      const to = ta?.selectionEnd ?? at;
      const edit = insertNativeInlineImages(current, at, to, [attachment]);
      // Record before onChange schedules the render that replaces this textarea.
      onInlineImageInsertion?.(edit.caret, attachment.pending === true);
      if (ta) {
        writeNativeEdit(ta, {
          value: edit.value,
          from: edit.caret,
          to: edit.caret,
        });
      }
      onChange(edit.value);
    },
    insertImages: (
      attachments: Attachment[],
      capturedSelection?: ComposerEditorSelection,
    ): void => {
      if (attachments.length === 0) return;
      const ta = inputRef.current;
      // Read the live DOM value: image conversion is asynchronous and React's
      // render-time `value` may lag text entered while the clipboard was read.
      const current = ta?.value ?? value;
      const anchor = capturedSelection?.anchor ??
        ta?.selectionStart ?? current.length;
      const head = capturedSelection?.head ?? ta?.selectionEnd ?? anchor;
      const at = Math.min(anchor, head);
      const to = Math.max(anchor, head);
      const edit = insertNativeInlineImages(current, at, to, attachments);
      // CM6's initial EditorState consumes this exact selection in the same
      // promotion commit. Defaulting to 0 strands the caret before the image.
      onInlineImageInsertion?.(
        edit.caret,
        attachments.some((attachment) => attachment.pending === true),
      );
      if (ta) {
        // The accessory button prevents pointer-down focus transfer, but iOS may
        // briefly project BODY while showing its native paste affordance. Restore
        // the same textarea before the promotion render so the replacement CM6
        // inherits the still-open keyboard rather than needing delayed refocus.
        ta.focus();
        writeNativeEdit(ta, {
          value: edit.value,
          from: edit.caret,
          to: edit.caret,
        });
      }
      onChange(edit.value);
    },
    refreshImages: (): void => undefined,
    deleteImage: (): void => undefined,
    // The fullscreen touch editor deliberately remains a native textarea while
    // it has no inline-image widget. Apply the same literal Markdown operations
    // as CM6 without replacing the native first responder.
    wrap: (before: string, after: string): void => {
      const current = currentTextSelection();
      applyTextEdit(
        wrapNativeSelection(
          current.value,
          current.from,
          current.to,
          before,
          after,
        ),
      );
    },
    toggleWrap: (marker: string): void => {
      const current = currentTextSelection();
      applyTextEdit(
        toggleNativeWrap(current.value, current.from, current.to, marker),
      );
    },
    indent: (): void => {
      const current = currentTextSelection();
      applyTextEdit(
        indentNativeLines(current.value, current.from, current.to),
      );
    },
    outdent: (): void => {
      const current = currentTextSelection();
      applyTextEdit(
        outdentNativeLines(current.value, current.from, current.to),
      );
    },
    toggleLinePrefix: (prefix: string): void => {
      const current = currentTextSelection();
      applyTextEdit(
        toggleNativeLinePrefix(current.value, current.from, current.to, prefix),
      );
    },
    cycleHeading: (): void => {
      const current = currentTextSelection();
      applyTextEdit(
        cycleNativeHeading(current.value, current.from, current.to),
      );
    },
    setHeading: (level: number): void => {
      const current = currentTextSelection();
      applyTextEdit(
        setNativeHeading(current.value, current.from, current.to, level),
      );
    },
    toggleCheckbox: (): void => {
      const current = currentTextSelection();
      const edit = toggleNativeCheckbox(
        current.value,
        current.from,
        current.to,
      );
      if (edit) applyTextEdit(edit);
    },
    insertLink: (): void => {
      const current = currentTextSelection();
      applyTextEdit(
        insertNativeLink(current.value, current.from, current.to),
      );
    },
    insertCodeBlock: (): void => {
      const current = currentTextSelection();
      applyTextEdit(
        insertNativeCodeBlock(current.value, current.from, current.to),
      );
    },
    undo: (): void => {
      const ta = inputRef.current;
      if (!ta) return;
      ta.focus();
      document.execCommand("undo");
      lastNativeValueRef.current = ta.value;
      onChange(ta.value);
      publishSelection(ta);
    },
    redo: (): void => {
      const ta = inputRef.current;
      if (!ta) return;
      ta.focus();
      document.execCommand("redo");
      lastNativeValueRef.current = ta.value;
      onChange(ta.value);
      publishSelection(ta);
    },
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
          onClick={(): void => applyOption(o)}
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
      ref={rootRef}
      data-mobile-native-editor
      data-mobile-pager-ignore
      data-mobile-drawer-ignore
      sx={{
        position: "relative",
        // `expanded` (fullscreen compose / edit sheet): fill the sheet so the
        // textarea is a full-height writing canvas, not a fixed block with blank
        // space below it.
        ...(expanded &&
          { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }),
      }}
    >
      {popup}
      <Box
        component="textarea"
        ref={inputRef}
        data-mobile-native-textarea="true"
        aria-label={placeholder ?? "Message the agent"}
        // The native textarea owns its live value/selection on touch surfaces.
        // React still receives every change through onChange, while the layout
        // effect above handles only genuine external value transitions.
        defaultValue={value}
        onChange={(e): void => {
          const command = selectedSlashCommandRef.current;
          if (command) {
            const next = e.target.value;
            const rest = next.slice(command.length + 1);
            if (
              !next.startsWith(`/${command}`) ||
              (rest !== "" && !/^\s/u.test(rest))
            ) {
              selectedSlashCommandRef.current = null;
            }
          }
          lastNativeValueRef.current = e.target.value;
          onChange(e.target.value);
          sync(
            e.target.value,
            e.target.selectionStart ?? e.target.value.length,
          );
          rememberSelection(e.target as HTMLTextAreaElement);
        }}
        onSelect={(e): void => {
          const ta = e.target as HTMLTextAreaElement;
          sync(ta.value, ta.selectionStart ?? ta.value.length);
          publishSelection(ta);
        }}
        onKeyDown={(e): void => {
          if (isImeKeyEvent(e.nativeEvent)) return;
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
        onBlur={(e): void => {
          const ta = e.currentTarget as HTMLTextAreaElement;
          rememberSelection(ta);
          setTrigger(null);
          if (!expanded) {
            fitNativeTextareaExactly(ta);
            syncNativeScrollable(ta);
          }
        }}
        onPaste={(e): void => {
          const files = clipboardFiles(e.clipboardData);
          reportMobileNativePasteEvent({
            surface: "textarea",
            clipboard: e.clipboardData,
            fileCount: files.length,
            consumed: files.length > 0 && !!onPasteFiles,
          });
          if (files.length > 0 && onPasteFiles) {
            e.preventDefault();
            onPasteFiles(files);
            return;
          }
          // iOS Safari / PWA often delivers keyboard-shelf photos ("拷贝的
          // 图片") with an empty DataTransfer on a <textarea>. The same
          // user gesture can still read the web clipboard port. Native
          // shell never needs this: its accessory button uses the
          // pasteboard bridge, and UIKit paste already carries files.
          if (!onPasteFiles) return;
          const types = e.clipboardData ? Array.from(e.clipboardData.types) : [];
          const looksLikeImage = types.some((type) =>
            type === "Files" || type.startsWith("image/")
          );
          // Empty types: let the textarea keep ordinary text paste, and
          // speculatively read images on the same gesture.
          if (!looksLikeImage) {
            if (types.length === 0) {
              void readWebClipboard().then((contents) => {
                if (contents.files.length > 0) onPasteFiles(contents.files);
              });
            }
            return;
          }
          e.preventDefault();
          void readWebClipboard().then((contents) => {
            if (contents.files.length > 0) onPasteFiles(contents.files);
          });
        }}
        placeholder={placeholder}
        disabled={disabled}
        autoFocus={autoFocus}
        rows={1}
        sx={{
          // This must remain a literal native textarea. MUI TextareaAutosize's
          // input wrapper calls setSelectionRange after every trailing newline
          // (a Chromium workaround), which races iOS's UIKit-owned caret overlay
          // and leaves it painted on the previous line. Styling through Box is
          // safe: Box does not wrap or replace the textarea's input transaction.
          boxSizing: "border-box",
          display: "block",
          flex: expanded ? 1 : undefined,
          width: "100%",
          minWidth: 0,
          minHeight: expanded ? "100%" : 48,
          height: expanded ? "100%" : "auto",
          maxHeight: "100%",
          m: 0,
          padding: "8.5px 14px",
          ...(endInset > 0 && { paddingRight: `${String(14 + endInset)}px` }),
          resize: "none",
          appearance: "none",
          WebkitAppearance: "none",
          outline: "none",
          border: borderless ? 0 : "1px solid",
          borderColor: borderless ? "transparent" : "divider",
          borderRadius: borderless ? 0 : 1,
          bgcolor: "transparent",
          color: disabled ? "text.disabled" : "text.primary",
          WebkitTextFillColor: disabled ? "text.disabled" : "currentColor",
          fontFamily: "inherit",
          fontSize: "1rem",
          fontWeight: "inherit",
          lineHeight: "var(--cowboy-reading-line-height, 1.5)",
          overflowX: "hidden",
          overflowY: nativeScrollable ? "auto" : "hidden",
          caretColor: "primary.main",
          "&::placeholder": { color: "text.secondary", opacity: 1 },
          "&:focus": borderless ? undefined : { borderColor: "primary.main" },
        }}
      />
    </Box>
  );
});
