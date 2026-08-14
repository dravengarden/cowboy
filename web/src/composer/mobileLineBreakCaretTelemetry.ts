import { type EditorView, ViewPlugin } from "@codemirror/view";
import { flushObservability, reportClientLog } from "../observability";

export function isMobileLineBreakInput(
  inputType: string | undefined,
): boolean {
  return inputType === "insertLineBreak" || inputType === "insertParagraph";
}

export function isMobileCaretGeometryInput(
  inputType: string | undefined,
): boolean {
  return isMobileLineBreakInput(inputType) ||
    inputType === "deleteContentBackward" ||
    inputType === "deleteContentForward";
}

export type MobileCaretNodeRelation =
  | "missing"
  | "content_root"
  | "caret_anchor"
  | "active_line"
  | "inside_active_line"
  | "other_line"
  | "image_widget"
  | "other";

export function mobileCaretNodeRelation(
  node: Node | null,
  contentDOM: HTMLElement,
  activeLine: HTMLElement | null,
): MobileCaretNodeRelation {
  if (node === null) return "missing";
  if (node === contentDOM) return "content_root";
  const element = node instanceof Element ? node : node.parentElement;
  if (element?.closest(".cm-mobile-empty-line-caret-anchor")) {
    return "caret_anchor";
  }
  if (activeLine !== null) {
    if (node === activeLine) return "active_line";
    if (activeLine.contains(node)) return "inside_active_line";
  }
  if (element?.closest(".cm-inline-image-widget")) return "image_widget";
  if (element?.closest(".cm-line")) return "other_line";
  return "other";
}

function rounded(value: number): number {
  return Math.round(value * 10) / 10;
}

/**
 * Read-only, bounded telemetry for the physical-iOS line-break path. It records
 * no document text, attachment ids, names, or clipboard data. The probe exists
 * because Simulator and physical WKWebView use different native caret nodes.
 */
function reportMobileCaretState(
  view: EditorView,
  sequence: number,
  phase: "frame" | "settled",
): void {
  const content = view.contentDOM;
  if (!content.querySelector(".cm-inline-image-widget")) return;
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
  const lineElements = Array.from(
    content.querySelectorAll<HTMLElement>(".cm-line"),
  );
  const rootImageWidgets =
    Array.from(content.children).filter((element) =>
      element.classList.contains("cm-inline-image-widget")
    ).length;

  reportClientLog(
    "info",
    "mobile_caret_line_break",
    "Mobile composer caret state after line break",
    {
      sequence,
      phase,
      focus_owned: content.ownerDocument.activeElement === content,
      composing: view.composing,
      state_empty: stateSelection.empty,
      state_head: stateSelection.head,
      state_line: stateLine.number,
      document_lines: view.state.doc.lines,
      active_line_empty: activeLine?.textContent === "",
      active_line_index: activeLine === null
        ? -1
        : lineElements.indexOf(activeLine),
      anchor_relation: mobileCaretNodeRelation(
        selection?.anchorNode ?? null,
        content,
        activeLine,
      ),
      focus_relation: mobileCaretNodeRelation(
        selection?.focusNode ?? null,
        content,
        activeLine,
      ),
      anchor_offset: selection?.anchorOffset ?? -1,
      focus_offset: selection?.focusOffset ?? -1,
      native_collapsed: selection?.isCollapsed ?? false,
      root_image_widgets: rootImageWidgets,
      nested_image_widgets: content.querySelectorAll(
        ".cm-line .cm-inline-image-widget",
      ).length,
      caret_anchor_widgets: content.querySelectorAll(
        ".cm-mobile-empty-line-caret-anchor",
      ).length,
      line_top: lineRect ? rounded(lineRect.top - contentRect.top) : -1,
      line_height: lineRect ? rounded(lineRect.height) : -1,
      caret_top: rangeRect ? rounded(rangeRect.top - contentRect.top) : -1,
      caret_height: rangeRect ? rounded(rangeRect.height) : -1,
    },
  );
}

/** Touch-only observability for the image-bearing CM6 path. */
export const mobileLineBreakCaretTelemetry = ViewPlugin.fromClass(
  class {
    private frame = 0;
    private timer = 0;
    private sequence = 0;

    constructor(readonly view: EditorView) {}

    schedule(): void {
      // `beforeinput` and `input` describe one Return. Requeue that same sample
      // while its frame is pending instead of consuming two sequence slots.
      if (this.frame !== 0) {
        globalThis.cancelAnimationFrame(this.frame);
      } else {
        if (this.sequence >= 4) return;
        this.sequence += 1;
      }
      const sequence = this.sequence;
      if (this.timer !== 0) {
        globalThis.clearTimeout(this.timer);
        this.timer = 0;
      }
      this.frame = globalThis.requestAnimationFrame(() => {
        this.frame = 0;
        reportMobileCaretState(this.view, sequence, "frame");
        this.timer = globalThis.setTimeout(() => {
          this.timer = 0;
          reportMobileCaretState(this.view, sequence, "settled");
          void flushObservability();
        }, 120);
      });
    }

    destroy(): void {
      if (this.frame !== 0) globalThis.cancelAnimationFrame(this.frame);
      if (this.timer !== 0) globalThis.clearTimeout(this.timer);
    }
  },
  {
    eventHandlers: {
      beforeinput(event: InputEvent): void {
        if (isMobileCaretGeometryInput(event.inputType)) this.schedule();
      },
      input(event: Event): void {
        const inputEvent = event as InputEvent;
        if (isMobileCaretGeometryInput(inputEvent.inputType)) this.schedule();
      },
    },
  },
);
