import { type EditorView, ViewPlugin } from "@codemirror/view";

export interface MobileEmptyLineCaretSnapshot {
  activeElement: Element | null;
  anchorNode: Node | null;
  focusNode: Node | null;
  composing: boolean;
  lineEmpty: boolean;
}

/**
 * WebKit may keep the native Selection on CM6's content root after Return
 * creates an empty line below a block decoration. CM6's logical selection and
 * active-line decoration have already advanced, but UIKit can only paint the
 * caret at the stale root position until a character creates a text node.
 */
export function shouldRepairMobileEmptyLineCaret(
  snapshot: MobileEmptyLineCaretSnapshot,
  contentDOM: HTMLElement,
): boolean {
  return snapshot.activeElement === contentDOM &&
    snapshot.anchorNode === contentDOM &&
    snapshot.focusNode === contentDOM &&
    !snapshot.composing &&
    snapshot.lineEmpty;
}

export function isLineBreakInput(inputType: string | undefined): boolean {
  return inputType === "insertLineBreak" || inputType === "insertParagraph";
}

function repairEmptyLineCaret(view: EditorView): void {
  const lineElement = view.contentDOM.querySelector<HTMLElement>(
    ".cm-line.cm-activeLine",
  );
  if (!lineElement || !view.state.selection.main.empty) return;
  const selection = view.contentDOM.ownerDocument.getSelection();
  if (!selection) return;
  if (
    !shouldRepairMobileEmptyLineCaret(
      {
        activeElement: view.contentDOM.ownerDocument.activeElement,
        anchorNode: selection.anchorNode,
        focusNode: selection.focusNode,
        composing: view.composing,
        lineEmpty: lineElement.textContent === "",
      },
      view.contentDOM,
    )
  ) return;

  selection.collapse(lineElement, 0);
}

/** Touch-only CM6 repair for the inline-image editor path. */
export const mobileEmptyLineCaret = ViewPlugin.fromClass(
  class {
    private frame = 0;

    constructor(readonly view: EditorView) {}

    schedule(): void {
      if (this.frame !== 0) globalThis.cancelAnimationFrame(this.frame);
      this.frame = globalThis.requestAnimationFrame(() => {
        this.frame = 0;
        repairEmptyLineCaret(this.view);
      });
    }

    destroy(): void {
      if (this.frame !== 0) globalThis.cancelAnimationFrame(this.frame);
    }
  },
  {
    eventHandlers: {
      beforeinput(event: InputEvent): void {
        if (!isLineBreakInput(event.inputType)) return;
        this.schedule();
      },
      input(event: Event): void {
        const inputEvent = event as InputEvent;
        if (!isLineBreakInput(inputEvent.inputType)) return;
        // Software keyboards may not commit CM6's replacement line until the
        // input event. Requeue over the beforeinput frame so the last scheduled
        // repair always sees the committed `.cm-activeLine` DOM.
        this.schedule();
      },
    },
  },
);
