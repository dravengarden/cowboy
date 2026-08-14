import { type EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { getComposerDebugSetting } from "../composerDebugSetting";
import { flushObservability, reportClientLog } from "../observability";
import { isLoneImageTokenLine } from "./inlineImageCaretPolicy";
import {
  type ComposerInputDebugSample,
  emptyComposerInputDebugRate,
  safeComposerDebugKey,
  shouldSampleComposerInputDebug,
  textareaLineMetrics,
} from "./composerInputDebugPolicy";
import { mobileCaretNodeRelation } from "./mobileLineBreakCaretTelemetry";

export type {
  ComposerInputDebugEditor,
  ComposerInputDebugSample,
  ComposerInputDebugSurface,
} from "./composerInputDebugPolicy";
export { safeComposerDebugKey } from "./composerInputDebugPolicy";

const FLUSH_DEBOUNCE_MS = 250;
const rate = emptyComposerInputDebugRate();
let flushTimer = 0;

export function composerInputDebugEnabled(): boolean {
  return getComposerDebugSetting();
}

export function reportComposerDebugModeChanged(enabled: boolean): void {
  reportClientLog(
    "info",
    "composer_debug_mode",
    enabled
      ? "Composer input debug mode enabled"
      : "Composer input debug mode disabled",
    { enabled, debug_mode: enabled },
  );
  void flushObservability();
}

function rounded(value: number): number {
  return Math.round(value * 10) / 10;
}

function scheduleDebugFlush(): void {
  if (flushTimer !== 0) return;
  flushTimer = globalThis.setTimeout(() => {
    flushTimer = 0;
    void flushObservability();
  }, FLUSH_DEBOUNCE_MS);
}

function emit(sample: ComposerInputDebugSample): void {
  reportClientLog(
    "debug",
    "composer_input_debug",
    "Composer input debug sample",
    { ...sample, debug_mode: true },
  );
  scheduleDebugFlush();
}

function viewportHeight(): number {
  return globalThis.visualViewport?.height ?? globalThis.innerHeight ?? -1;
}

function record(sample: () => ComposerInputDebugSample): void {
  const gate = shouldSampleComposerInputDebug(
    composerInputDebugEnabled(),
    Date.now(),
    rate,
  );
  if (!gate.sample) return;
  const next = sample();
  next.dropped_events = gate.dropped;
  emit(next);
}

export function textareaDebugSample(
  value: string,
  head: number,
  collapsed: boolean,
  phase: string,
  inputType: string,
  key: string,
  trusted: boolean,
  defaultPrevented: boolean,
  composing: boolean,
  focusOwned: boolean,
  caretHeight: number,
  caretTop: number,
  lineHeight: number,
  viewport: number,
): ComposerInputDebugSample {
  const metrics = textareaLineMetrics(value, head);
  return {
    surface: "mobile",
    editor: "textarea",
    phase,
    input_type: inputType,
    key: safeComposerDebugKey(key),
    trusted,
    default_prevented: defaultPrevented,
    composing,
    focus_owned: focusOwned,
    target_relation: "textarea",
    state_head: head,
    state_line: metrics.line,
    document_lines: metrics.lines,
    line_length: metrics.lineLength,
    previous_line_is_image: false,
    has_image_widget: false,
    caret_anchor_widgets: 0,
    native_collapsed: collapsed,
    caret_height: caretHeight,
    caret_top: caretTop,
    line_height: lineHeight,
    line_top: 0,
    visual_viewport_height: viewport,
    dropped_events: 0,
  };
}

function cm6DebugSample(
  view: EditorView,
  surface: "desktop" | "mobile",
  phase: string,
  event: Event | null,
): ComposerInputDebugSample {
  const content = view.contentDOM;
  const activeLine = content.querySelector<HTMLElement>(
    ".cm-line.cm-activeLine",
  );
  const selection = content.ownerDocument.getSelection();
  const contentRect = content.getBoundingClientRect();
  const lineRect = activeLine?.getBoundingClientRect();
  const rangeRect = selection?.rangeCount
    ? selection.getRangeAt(0).getBoundingClientRect()
    : undefined;
  const stateSelection = view.state.selection.main;
  const stateLine = view.state.doc.lineAt(stateSelection.head);
  const previousLineIsImage = stateLine.number > 1 &&
    isLoneImageTokenLine(view.state.doc.line(stateLine.number - 1).text);
  const input = event as InputEvent | null;
  const keyEvent = event as KeyboardEvent | null;
  return {
    surface,
    editor: "cm6",
    phase,
    input_type: input?.inputType ?? keyEvent?.key ?? phase,
    key: safeComposerDebugKey(keyEvent?.key),
    trusted: event?.isTrusted ?? false,
    default_prevented: event?.defaultPrevented ?? false,
    composing: view.composing,
    focus_owned: content.ownerDocument.activeElement === content,
    target_relation: event
      ? mobileCaretNodeRelation(
        event.target instanceof Node ? event.target : null,
        content,
        activeLine,
      )
      : mobileCaretNodeRelation(
        selection?.anchorNode ?? null,
        content,
        activeLine,
      ),
    state_head: stateSelection.head,
    state_line: stateLine.number,
    document_lines: view.state.doc.lines,
    line_length: stateLine.length,
    previous_line_is_image: previousLineIsImage,
    has_image_widget: content.querySelector(".cm-inline-image-widget") !== null,
    caret_anchor_widgets: content.querySelectorAll(
      ".cm-mobile-empty-line-caret-anchor",
    ).length,
    native_collapsed: selection?.isCollapsed ?? stateSelection.empty,
    caret_height: rangeRect ? rounded(rangeRect.height) : -1,
    caret_top: rangeRect ? rounded(rangeRect.top - contentRect.top) : -1,
    line_height: lineRect ? rounded(lineRect.height) : -1,
    line_top: lineRect ? rounded(lineRect.top - contentRect.top) : -1,
    visual_viewport_height: rounded(viewportHeight()),
    dropped_events: 0,
  };
}

const DEBUG_EVENTS = [
  "beforeinput",
  "input",
  "keydown",
  "keyup",
  "compositionstart",
  "compositionupdate",
  "compositionend",
  "paste",
  "focus",
  "blur",
] as const;

export function attachComposerInputDebug(
  element: HTMLTextAreaElement,
): () => void {
  const onEvent = (event: Event): void => {
    record(() => {
      const input = event as InputEvent;
      const keyEvent = event as KeyboardEvent;
      const selection = element.ownerDocument.getSelection();
      const rangeRect = selection?.rangeCount
        ? selection.getRangeAt(0).getBoundingClientRect()
        : undefined;
      const box = element.getBoundingClientRect();
      const head = element.selectionStart ?? 0;
      return textareaDebugSample(
        element.value,
        head,
        element.selectionStart === element.selectionEnd,
        event.type,
        input.inputType ?? keyEvent.key ?? event.type,
        keyEvent.key ?? "",
        event.isTrusted,
        event.defaultPrevented,
        event.type.startsWith("composition") || Boolean(input.isComposing),
        element.ownerDocument.activeElement === element,
        rangeRect ? rounded(rangeRect.height) : -1,
        rangeRect ? rounded(rangeRect.top - box.top) : -1,
        rounded(box.height),
        rounded(viewportHeight()),
      );
    });
  };
  for (const type of DEBUG_EVENTS) {
    element.addEventListener(type, onEvent);
  }
  return () => {
    for (const type of DEBUG_EVENTS) {
      element.removeEventListener(type, onEvent);
    }
  };
}

export function composerInputDebugExtension(
  surface: "desktop" | "mobile",
) {
  return ViewPlugin.fromClass(
    class {
      constructor(readonly view: EditorView) {}

      update(update: ViewUpdate): void {
        if (!update.selectionSet && !update.docChanged) return;
        record(() =>
          cm6DebugSample(
            this.view,
            surface,
            update.docChanged ? "cm6_doc" : "cm6_selection",
            null,
          )
        );
      }
    },
    {
      eventHandlers: {
        beforeinput(event: InputEvent): void {
          record(() => cm6DebugSample(this.view, surface, "beforeinput", event));
        },
        input(event: Event): void {
          record(() => cm6DebugSample(this.view, surface, "input", event));
        },
        keydown(event: KeyboardEvent): void {
          if (safeComposerDebugKey(event.key) === "char") return;
          record(() => cm6DebugSample(this.view, surface, "keydown", event));
        },
        compositionstart(event: Event): void {
          record(() =>
            cm6DebugSample(this.view, surface, "compositionstart", event)
          );
        },
        compositionend(event: Event): void {
          record(() =>
            cm6DebugSample(this.view, surface, "compositionend", event)
          );
        },
        paste(event: Event): void {
          record(() => cm6DebugSample(this.view, surface, "paste", event));
        },
        focus(event: Event): void {
          record(() => cm6DebugSample(this.view, surface, "focus", event));
        },
        blur(event: Event): void {
          record(() => cm6DebugSample(this.view, surface, "blur", event));
        },
      },
    },
  );
}
