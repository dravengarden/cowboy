import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import ContentPasteRounded from "@mui/icons-material/ContentPasteRounded";
import {
  Box,
  ButtonBase,
  CircularProgress,
  Paper,
  TextField,
  Typography,
} from "@mui/material";
import type { ComposerEditorHandle } from "./ComposerEditor";
import { type Attachment, clipboardFiles } from "./attachments";
import { readComposerClipboard } from "./clipboard";
import {
  BLANK_CANVAS_LONG_PRESS_MS,
  isBlankCanvasPress,
  longPressMoved,
  measureTextareaContentHeight,
} from "./composer/mobileBlankCanvasPaste";
import { hasDraftMod, hasSendMod } from "./platform";
import type { AvailableCommand } from "./protocol";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { navigationHaptic } from "./haptic";
import { useNetworkActionState } from "./NetworkActionFeedback";
import {
  cycleNativeHeading,
  indentNativeLines,
  insertNativeCodeBlock,
  insertNativeLink,
  type NativeTextEdit,
  outdentNativeLines,
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

interface PendingBlankCanvasPress {
  identifier: number;
  clientX: number;
  clientY: number;
  textareaScrollTop: number;
  fired: boolean;
}

interface BlankPasteAnchor {
  left: number;
  top: number;
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
    onEscape?: () => boolean;
    onPasteFiles?: (files: File[]) => void;
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
    onEscape,
    onPasteFiles,
    onSelectionChange,
    endInset = 0,
    borderless = false,
    expanded = false,
  },
  ref,
): React.JSX.Element {
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const blankPressTimerRef = useRef<number | null>(null);
  const pendingBlankPressRef = useRef<PendingBlankCanvasPress | null>(null);
  const [blankPasteAnchor, setBlankPasteAnchor] = useState<
    BlankPasteAnchor | null
  >(null);
  const pasteFeedback = useNetworkActionState();
  const selectedSlashCommandRef = useRef<string | null>(null);
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

  const publishSelection = (ta: HTMLTextAreaElement): void => {
    onSelectionChange?.(ta.selectionStart !== ta.selectionEnd);
  };

  const currentTextSelection = (): {
    value: string;
    from: number;
    to: number;
  } => {
    const ta = inputRef.current;
    const current = ta?.value ?? value;
    const from = ta?.selectionStart ?? current.length;
    const to = ta?.selectionEnd ?? from;
    return { value: current, from, to };
  };

  const cancelBlankPressTimer = (): void => {
    if (blankPressTimerRef.current != null) {
      globalThis.clearTimeout(blankPressTimerRef.current);
      blankPressTimerRef.current = null;
    }
  };

  useEffect(() => {
    return (): void => cancelBlankPressTimer();
  }, []);

  useEffect(() => {
    if (!blankPasteAnchor) return undefined;
    const closeOnOutsidePointer = (event: PointerEvent): void => {
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("[data-mobile-blank-paste-menu]")
      ) return;
      setBlankPasteAnchor(null);
    };
    document.addEventListener("pointerdown", closeOnOutsidePointer, true);
    return (): void => {
      document.removeEventListener("pointerdown", closeOnOutsidePointer, true);
    };
  }, [blankPasteAnchor]);

  const showBlankPasteMenu = (
    press: PendingBlankCanvasPress,
  ): void => {
    const root = rootRef.current;
    if (!root) return;
    const rect = root.getBoundingClientRect();
    const halfMenuWidth = 54;
    press.fired = true;
    setBlankPasteAnchor({
      left: Math.max(
        halfMenuWidth + 8,
        Math.min(rect.width - halfMenuWidth - 8, press.clientX - rect.left),
      ),
      top: Math.max(
        8,
        Math.min(rect.height - 52, press.clientY - rect.top - 54),
      ),
    });
    navigationHaptic();
  };

  const pasteFromBlankCanvas = async (): Promise<void> => {
    const selection = currentTextSelection();
    const content = await readComposerClipboard();
    if (content.files.length > 0) {
      if (!onPasteFiles) throw new Error("This composer cannot paste files");
      onPasteFiles(content.files);
      return;
    }
    if (!content.text) throw new Error("The clipboard is empty");

    const textarea = inputRef.current;
    const current = textarea?.value ?? selection.value;
    const from = Math.min(selection.from, current.length);
    const to = Math.min(selection.to, current.length);
    const next = current.slice(0, from) + content.text + current.slice(to);
    const caret = from + content.text.length;
    onChange(next);
    setTrigger(null);
    requestAnimationFrame(() => {
      const currentTextarea = inputRef.current;
      if (!currentTextarea) return;
      currentTextarea.setSelectionRange(caret, caret);
      publishSelection(currentTextarea);
    });
  };

  // Accessory buttons prevent pointer-down default, so the native textarea is
  // still UIKit's first responder when this runs. Commit literal Markdown to the
  // controlled value, then restore the transformed selection after React paints;
  // there is no delayed focus handoff and therefore no keyboard/menu reset.
  const applyTextEdit = (edit: NativeTextEdit): void => {
    inputRef.current?.focus();
    onChange(edit.value);
    setTrigger(null);
    requestAnimationFrame(() => {
      const ta = inputRef.current;
      if (!ta) return;
      ta.setSelectionRange(edit.from, edit.to);
      publishSelection(ta);
    });
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

  // Splice the chosen token in. NO .focus() here: the option row's mousedown
  // preventDefault kept the textarea focused (keyboard never dropped), so we only
  // move the caret (in a rAF, after the controlled value re-renders). Calling
  // focus() — especially deferred — is what caused the iOS phantom-keyboard gap.
  const applyOption = (option: PickerOption): void => {
    if (!trigger) return;
    const { apply } = option;
    const end = trigger.from + 1 + trigger.query.length;
    const next = value.slice(0, trigger.from) + apply + value.slice(end);
    onChange(next);
    selectedSlashCommandRef.current = option.slashCommand ?? null;
    setTrigger(null);
    const pos = trigger.from + apply.length;
    requestAnimationFrame(() => inputRef.current?.setSelectionRange(pos, pos));
  };

  useImperativeHandle(ref, () => ({
    focus: (): void => inputRef.current?.focus(),
    // The native touch textarea has no Vim state; Escape belongs to its host.
    escapeBelongsToApp: (): boolean => true,
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
      const at = ta?.selectionStart ?? value.length;
      const to = ta?.selectionEnd ?? at;
      const lineStart = value.lastIndexOf("\n", at - 1) + 1;
      const lead = at === lineStart ? "" : "\n";
      const label = attachment.name.replaceAll("]", "");
      const token = `${lead}![${label}](cowboy-att:${attachment.id})\n`;
      onChange(value.slice(0, at) + token + value.slice(to));
    },
    insertImages: (attachments: Attachment[]): void => {
      if (attachments.length === 0) return;
      const ta = inputRef.current;
      // Read the live DOM value: image conversion is asynchronous and React's
      // render-time `value` may lag text entered while the clipboard was read.
      const current = ta?.value ?? value;
      const at = ta?.selectionStart ?? current.length;
      const to = ta?.selectionEnd ?? at;
      const lineStart = current.lastIndexOf("\n", at - 1) + 1;
      const tokens = attachments.map((attachment, index) => {
        const lead = index === 0 && at !== lineStart ? "\n" : "";
        const label = attachment.name.replaceAll("]", "");
        return `${lead}![${label}](cowboy-att:${attachment.id})\n`;
      }).join("");
      onChange(current.slice(0, at) + tokens + current.slice(to));
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
      requestAnimationFrame(() => {
        onChange(ta.value);
        publishSelection(ta);
      });
    },
    redo: (): void => {
      const ta = inputRef.current;
      if (!ta) return;
      ta.focus();
      document.execCommand("redo");
      requestAnimationFrame(() => {
        onChange(ta.value);
        publishSelection(ta);
      });
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
      {blankPasteAnchor && (
        <Paper
          data-mobile-blank-paste-menu
          role="menu"
          elevation={5}
          sx={{
            position: "absolute",
            left: blankPasteAnchor.left,
            top: blankPasteAnchor.top,
            transform: "translateX(-50%)",
            zIndex: 6,
            border: "1px solid",
            borderColor: "divider",
            borderRadius: 2,
            overflow: "hidden",
            bgcolor: "background.paper",
            color: "text.primary",
            userSelect: "none",
            WebkitUserSelect: "none",
          }}
        >
          <ButtonBase
            role="menuitem"
            disabled={pasteFeedback.pending}
            disableRipple
            onPointerDown={(event): void => {
              // This menu is deliberately non-modal. Keep the textarea as
              // UIKit's first responder while the trusted tap reads clipboard.
              event.preventDefault();
              event.stopPropagation();
            }}
            onClick={(): void => {
              void pasteFeedback.run(pasteFromBlankCanvas).then((pasted) => {
                if (pasted) setBlankPasteAnchor(null);
              });
            }}
            sx={{
              minWidth: 108,
              minHeight: 44,
              px: 1.5,
              gap: 0.875,
              justifyContent: "flex-start",
              fontSize: "0.875rem",
              transition: "opacity 120ms ease, background-color 120ms ease",
              ...(pasteFeedback.pending && { opacity: 0.56 }),
              "&:active": { bgcolor: "action.selected" },
            }}
          >
            {pasteFeedback.progress
              ? <CircularProgress size={17} thickness={4.5} color="inherit" />
              : <ContentPasteRounded sx={{ fontSize: "1.125rem" }} />}
            <Box component="span">Paste</Box>
          </ButtonBase>
        </Paper>
      )}
      <TextField
        inputRef={inputRef}
        value={value}
        onChange={(e): void => {
          setBlankPasteAnchor(null);
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
          onChange(e.target.value);
          sync(
            e.target.value,
            e.target.selectionStart ?? e.target.value.length,
          );
        }}
        onSelect={(e): void => {
          setBlankPasteAnchor(null);
          const ta = e.target as HTMLTextAreaElement;
          sync(ta.value, ta.selectionStart ?? ta.value.length);
          publishSelection(ta);
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
        onTouchStart={(event): void => {
          setBlankPasteAnchor(null);
          cancelBlankPressTimer();
          pendingBlankPressRef.current = null;
          if (!expanded || disabled || event.touches.length !== 1) return;
          const textarea = inputRef.current;
          const touch = event.touches[0];
          if (!textarea || !touch) return;
          const rect = textarea.getBoundingClientRect();
          if (
            !isBlankCanvasPress({
              clientY: touch.clientY,
              textareaTop: rect.top,
              textareaHeight: rect.height,
              textareaScrollTop: textarea.scrollTop,
              naturalContentHeight: measureTextareaContentHeight(textarea),
            })
          ) return;

          const press: PendingBlankCanvasPress = {
            identifier: touch.identifier,
            clientX: touch.clientX,
            clientY: touch.clientY,
            textareaScrollTop: textarea.scrollTop,
            fired: false,
          };
          pendingBlankPressRef.current = press;
          blankPressTimerRef.current = globalThis.setTimeout(() => {
            blankPressTimerRef.current = null;
            if (
              pendingBlankPressRef.current === press &&
              inputRef.current?.scrollTop === press.textareaScrollTop
            ) {
              showBlankPasteMenu(press);
            }
          }, BLANK_CANVAS_LONG_PRESS_MS);
        }}
        onTouchMove={(event): void => {
          const press = pendingBlankPressRef.current;
          if (!press) return;
          const touch = Array.from(event.touches).find((candidate) =>
            candidate.identifier === press.identifier
          );
          if (
            !touch ||
            longPressMoved(
              press.clientX,
              press.clientY,
              touch.clientX,
              touch.clientY,
            )
          ) {
            cancelBlankPressTimer();
            pendingBlankPressRef.current = null;
          }
        }}
        onTouchEnd={(event): void => {
          const press = pendingBlankPressRef.current;
          if (
            press &&
            Array.from(event.changedTouches).some((touch) =>
              touch.identifier === press.identifier
            )
          ) {
            cancelBlankPressTimer();
            pendingBlankPressRef.current = null;
          }
        }}
        onTouchCancel={(): void => {
          // WebKit may cancel DOM touch delivery while its own long-press
          // recognizer decides there is no nearby text anchor. Keep an unmoved
          // blank-canvas timer alive so Cowboy's fallback still appears.
          if (pendingBlankPressRef.current?.fired) {
            pendingBlankPressRef.current = null;
          }
        }}
        onPaste={(e): void => {
          const files = clipboardFiles(e.clipboardData);
          if (files.length > 0 && onPasteFiles) {
            e.preventDefault();
            onPasteFiles(files);
          }
        }}
        placeholder={placeholder}
        disabled={disabled}
        autoFocus={autoFocus}
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
            // MUI normally keeps multiline padding on this non-editable DIV,
            // leaving only the 14px glyph line as the native textarea. A long
            // press in the visible top/bottom inset then lands outside the
            // editor and can dismiss UIKit's keyboard/menu. Put all hit area
            // padding on the textarea itself, matching the CM6 path.
            p: 0,
            alignItems: expanded ? "stretch" : "flex-start",
            ...(expanded && { height: "100%" }),
          },
          // `1rem` so the input tracks the reading font-size setting both up and
          // down; line-height follows the reading line-height var. No 16px floor
          // — the installed PWA disables focus-zoom via the viewport meta.
          "& .MuiInputBase-input": {
            boxSizing: "border-box",
            minHeight: 44,
            padding: "8.5px 14px",
            fontSize: "1rem",
            lineHeight: "var(--cowboy-reading-line-height, 1.5)",
            // Clear the overlaid send/kebab buttons at the bottom-right.
            ...(endInset > 0 && { paddingRight: `${String(14 + endInset)}px` }),
            ...(expanded &&
              { height: "100% !important", overflowY: "auto !important" }),
          },
          // Inside the composer's outlined Paper card the card draws the box, so
          // drop the TextField's own outline in all three states (rest/hover/focus).
          ...(borderless && {
            "& .MuiOutlinedInput-notchedOutline": { border: "none" },
            "&:hover .MuiOutlinedInput-notchedOutline": { border: "none" },
            "& .Mui-focused .MuiOutlinedInput-notchedOutline": {
              border: "none",
            },
          }),
        }}
      />
    </Box>
  );
});
