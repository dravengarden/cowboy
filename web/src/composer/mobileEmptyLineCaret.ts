import {
  type EditorState,
  StateEffect,
  StateField,
  type Transaction,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { flushObservability, reportClientLog } from "../observability";

export const MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS =
  "cm-mobile-empty-line-caret-anchor";

const setMobileEmptyLineCaretAnchor = StateEffect.define<boolean>();

/**
 * This character belongs only to a transient CM6 widget DOM node. It never
 * enters EditorState, the controlled composer value, history, or a message.
 */
class MobileEmptyLineCaretAnchorWidget extends WidgetType {
  override eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    const anchor = document.createElement("span");
    anchor.className = MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS;
    anchor.setAttribute("aria-hidden", "true");
    anchor.textContent = "\u200b";
    return anchor;
  }

  // CM6 normally makes widgets contenteditable=false and keeps Selection
  // outside them. This geometry-only widget must briefly own a native Range so
  // iOS WebKit measures the new empty line. The widget is removed one paint
  // later, before the next ordinary input transaction.
  get editable(): boolean {
    return true;
  }

  override ignoreEvent(): boolean {
    return true;
  }
}

function anchorDecoration(state: EditorState): DecorationSet {
  return Decoration.set([
    Decoration.widget({
      widget: new MobileEmptyLineCaretAnchorWidget(),
      side: 1,
    }).range(state.selection.main.head),
  ]);
}

const mobileEmptyLineCaretAnchorField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(anchors, transaction) {
    let enabled = anchors.size > 0;
    if (transaction.docChanged || transaction.selection !== undefined) {
      enabled = false;
    }
    for (const effect of transaction.effects) {
      if (effect.is(setMobileEmptyLineCaretAnchor)) enabled = effect.value;
    }
    return enabled ? anchorDecoration(transaction.state) : Decoration.none;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export interface MobileEmptyLineCaretRepairUpdate {
  docChanged: boolean;
  startLines: number;
  nextLines: number;
  directInput: boolean;
}

/** Only a direct keyboard input that adds a line can need caret repair. */
export function shouldRepairMobileEmptyLineCaret(
  update: MobileEmptyLineCaretRepairUpdate,
): boolean {
  return update.docChanged && update.directInput &&
    update.nextLines > update.startLines;
}

/** The repair is valid only for a collapsed caret on a genuinely empty line. */
export function isMobileEmptyLineCaretState(state: EditorState): boolean {
  const selection = state.selection.main;
  return selection.empty && state.doc.lineAt(selection.head).length === 0;
}

function rounded(value: number): number {
  return Math.round(value * 10) / 10;
}

function reportAnchorGeometry(view: EditorView, sequence: number): void {
  if (sequence <= 0) return;
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

  reportClientLog(
    "info",
    "mobile_caret_line_break_anchor",
    "Mobile composer caret received a transient geometry anchor",
    {
      sequence,
      focus_owned: content.ownerDocument.activeElement === content,
      composing: view.composing,
      state_head: stateSelection.head,
      state_line: view.state.doc.lineAt(stateSelection.head).number,
      document_lines: view.state.doc.lines,
      active_line_empty: activeLine?.textContent === "\u200b",
      anchor_dom_only: true,
      line_top: lineRect ? rounded(lineRect.top - contentRect.top) : -1,
      line_height: lineRect ? rounded(lineRect.height) : -1,
      caret_top: rangeRect ? rounded(rangeRect.top - contentRect.top) : -1,
      caret_height: rangeRect ? rounded(rangeRect.height) : -1,
    },
  );
  void flushObservability();
}

export function hasDirectKeyboardInput(
  transactions: readonly Transaction[],
): boolean {
  return transactions.some((transaction) =>
    transaction.isUserEvent("input") &&
    !transaction.isUserEvent("input.paste")
  );
}

/**
 * iOS WebKit can retain the native caret geometry of a preceding root-level
 * block image after Return, even though CM6's state and active line advanced.
 * Give WebKit one paint with a measurable, document-neutral text node at the
 * logical caret, then remove it on the following paint. The painted native
 * caret keeps the corrected line while subsequent input returns to ordinary
 * CM6 ownership.
 */
export const mobileEmptyLineCaretRepair = [
  mobileEmptyLineCaretAnchorField,
  ViewPlugin.fromClass(
    class {
      private mountFrame = 0;
      private placeFrame = 0;
      private releaseFrame = 0;
      private reportSequence = 0;

      constructor(readonly view: EditorView) {}

      private cancelFrames(): void {
        if (this.mountFrame !== 0) cancelAnimationFrame(this.mountFrame);
        if (this.placeFrame !== 0) cancelAnimationFrame(this.placeFrame);
        if (this.releaseFrame !== 0) cancelAnimationFrame(this.releaseFrame);
        this.mountFrame = this.placeFrame = this.releaseFrame = 0;
      }

      private disableAnchor(): void {
        const anchors = this.view.state.field(
          mobileEmptyLineCaretAnchorField,
          false,
        );
        if (anchors && anchors.size > 0) {
          this.view.dispatch({
            effects: setMobileEmptyLineCaretAnchor.of(false),
          });
        }
      }

      clear(): void {
        this.cancelFrames();
        this.disableAnchor();
      }

      private eligible(expectedHead?: number): boolean {
        const { state, contentDOM } = this.view;
        const selection = state.selection.main;
        return isMobileEmptyLineCaretState(state) &&
          (expectedHead === undefined || selection.head === expectedHead) &&
          !this.view.composing &&
          contentDOM.ownerDocument.activeElement === contentDOM &&
          contentDOM.querySelector(".cm-inline-image-widget") !== null;
      }

      private schedule(): void {
        this.clear();
        this.mountFrame = requestAnimationFrame(() => {
          this.mountFrame = 0;
          if (!this.eligible()) return;
          const expectedHead = this.view.state.selection.main.head;
          this.view.dispatch({
            effects: setMobileEmptyLineCaretAnchor.of(true),
          });

          this.placeFrame = requestAnimationFrame(() => {
            this.placeFrame = 0;
            if (!this.eligible(expectedHead)) {
              this.disableAnchor();
              return;
            }
            const anchor = this.view.contentDOM.querySelector<HTMLElement>(
              `.cm-line.cm-activeLine .${MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS}`,
            );
            const text = anchor?.firstChild;
            const selection = this.view.contentDOM.ownerDocument.getSelection();
            if (!text || text.nodeType !== Node.TEXT_NODE || !selection) {
              this.disableAnchor();
              return;
            }

            const range = this.view.contentDOM.ownerDocument.createRange();
            range.setStart(text, 0);
            range.collapse(true);
            selection.removeAllRanges();
            selection.addRange(range);

            const sequence = this.reportSequence < 4
              ? ++this.reportSequence
              : 0;
            reportAnchorGeometry(this.view, sequence);

            // One paint is sufficient to refresh WKWebView's native caret
            // cache. Do not leave an editable widget in the ordinary input
            // path, even though its character never belonged to EditorState.
            this.releaseFrame = requestAnimationFrame(() => {
              this.releaseFrame = 0;
              this.disableAnchor();
            });
          });
        });
      }

      update(update: ViewUpdate): void {
        if (
          shouldRepairMobileEmptyLineCaret({
            docChanged: update.docChanged,
            startLines: update.startState.doc.lines,
            nextLines: update.state.doc.lines,
            directInput: hasDirectKeyboardInput(update.transactions),
          })
        ) {
          this.schedule();
        }
      }

      destroy(): void {
        this.cancelFrames();
      }
    },
    {
      eventHandlers: {
        keydown(): void {
          this.clear();
        },
        compositionstart(): void {
          this.clear();
        },
        pointerdown(): void {
          this.clear();
        },
        touchstart(): void {
          this.clear();
        },
        blur(): void {
          this.clear();
        },
      },
    },
  ),
];
