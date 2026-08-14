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
import { reportClientLog } from "../observability";

export const MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS =
  "cm-mobile-empty-line-caret-anchor";

const setMobileEmptyLineCaretAnchor = StateEffect.define<boolean>();

/**
 * Transient-looking only to the document: this character lives in widget DOM.
 * It never enters EditorState, React value, history, or a sent message.
 * Physical WKWebView only paints a native caret while a real text node exists
 * at the logical empty line. Removing it after one paint put the Range back
 * to height 0, so the widget stays until the next real input.
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

  get editable(): boolean {
    return true;
  }

  override ignoreEvent(event: Event): boolean {
    return event.type !== "beforeinput" && event.type !== "paste" &&
      event.type !== "compositionstart";
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
    // A document change replaces the empty line. Do not treat a Selection
    // sync from the native Range as a reason to drop the only measurable
    // text node — that is what made the one-paint repair revert.
    if (transaction.docChanged) enabled = false;
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
  directDelete: boolean;
}

export function shouldRepairMobileEmptyLineCaret(
  update: MobileEmptyLineCaretRepairUpdate,
): boolean {
  if (!update.docChanged || update.nextLines === update.startLines) {
    return false;
  }
  return update.directInput || update.directDelete;
}

export function isMobileEmptyLineCaretState(state: EditorState): boolean {
  const selection = state.selection.main;
  return selection.empty && state.doc.lineAt(selection.head).length === 0;
}

export function hasDirectKeyboardInput(
  transactions: readonly Transaction[],
): boolean {
  return transactions.some((transaction) =>
    transaction.isUserEvent("input") &&
    !transaction.isUserEvent("input.paste")
  );
}

export function hasDirectDelete(
  transactions: readonly Transaction[],
): boolean {
  return transactions.some((transaction) =>
    transaction.isUserEvent("delete.backward") ||
    transaction.isUserEvent("delete.forward") ||
    transaction.isUserEvent("delete")
  );
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
    "Mobile composer caret received a durable geometry anchor",
    {
      sequence,
      focus_owned: content.ownerDocument.activeElement === content,
      composing: view.composing,
      state_head: stateSelection.head,
      state_line: view.state.doc.lineAt(stateSelection.head).number,
      document_lines: view.state.doc.lines,
      active_line_empty: activeLine?.textContent === "\u200b",
      anchor_dom_only: true,
      persistent_until_input: true,
      line_top: lineRect ? rounded(lineRect.top - contentRect.top) : -1,
      line_height: lineRect ? rounded(lineRect.height) : -1,
      caret_top: rangeRect ? rounded(rangeRect.top - contentRect.top) : -1,
      caret_height: rangeRect ? rounded(rangeRect.height) : -1,
    },
  );
}

export const mobileEmptyLineCaretRepair = [
  mobileEmptyLineCaretAnchorField,
  ViewPlugin.fromClass(
    class {
      private mountTimer = 0;
      private placeFrame = 0;
      private reportSequence = 0;

      constructor(readonly view: EditorView) {}

      private cancelFrames(): void {
        if (this.mountTimer !== 0) {
          globalThis.clearTimeout(this.mountTimer);
          this.mountTimer = 0;
        }
        if (this.placeFrame !== 0) cancelAnimationFrame(this.placeFrame);
        this.placeFrame = 0;
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

      private placeNativeCaret(): void {
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
        const sequence = this.reportSequence < 6 ? ++this.reportSequence : 0;
        reportAnchorGeometry(this.view, sequence);
      }

      private schedule(): void {
        this.cancelFrames();
        // Never dispatch from `update()`: CodeMirror rejects nested updates,
        // which is why v1249 mounted zero anchors on the physical phone.
        this.mountTimer = globalThis.setTimeout(() => {
          this.mountTimer = 0;
          if (!this.eligible()) return;
          const expectedHead = this.view.state.selection.main.head;
          const anchors = this.view.state.field(
            mobileEmptyLineCaretAnchorField,
            false,
          );
          if (!anchors || anchors.size === 0) {
            this.view.dispatch({
              effects: setMobileEmptyLineCaretAnchor.of(true),
            });
          }
          this.placeFrame = requestAnimationFrame(() => {
            this.placeFrame = 0;
            if (!this.eligible(expectedHead)) {
              this.disableAnchor();
              return;
            }
            this.placeNativeCaret();
          });
        }, 0);
      }

      update(update: ViewUpdate): void {
        if (
          shouldRepairMobileEmptyLineCaret({
            docChanged: update.docChanged,
            startLines: update.startState.doc.lines,
            nextLines: update.state.doc.lines,
            directInput: hasDirectKeyboardInput(update.transactions),
            directDelete: hasDirectDelete(update.transactions),
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
        beforeinput(event: InputEvent): void {
          // Return/Backspace must keep the current paint; docChanged already
          // drops the old widget. Tearing down on every key was the jank.
          if (
            event.inputType === "insertText" ||
            event.inputType === "insertCompositionText" ||
            event.inputType === "insertFromPaste"
          ) {
            this.clear();
          }
        },
        compositionstart(): void {
          this.clear();
        },
        paste(): void {
          this.clear();
        },
        blur(): void {
          this.clear();
        },
      },
    },
  ),
];
