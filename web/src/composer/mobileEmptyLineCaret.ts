import {
  type EditorState,
  StateEffect,
  StateField,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import {
  emptyLinePositionsAfterImages,
  selectionOnEmptyLineAfterImage,
} from "./inlineImageCaretPolicy";
import { reportClientLog } from "../observability";

export const MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS =
  "cm-mobile-empty-line-caret-anchor";

const setHideMobileEmptyLineCaretForIme = StateEffect.define<boolean>();

/**
 * Document-neutral only: this character lives in widget DOM. It never enters
 * EditorState, React value, history, or a sent message.
 *
 * Physical v1250 showed the late-mount repair was the wrong trigger. First
 * Return after paste still had no text node on the image-adjacent landing
 * line, so WKWebView kept the UIKit caret on the thumbnail. The widget then
 * appeared on the *next* empty line and ate the second Return as a native
 * <br> (line_height 28, document_lines unchanged). Backspace onto that same
 * landing line dropped the widget on docChanged and remounted it too late.
 *
 * Keep one editable U+200B on every empty line whose previous line is a
 * block image, in the same transaction that creates that line — including
 * paste. Do not decorate other empty lines. Mapping the DOM selection into
 * a newly appeared landing node only routes the next key; it is not the
 * Return/Backspace animation.
 */
class MobileEmptyLineCaretAnchorWidget extends WidgetType {
  override eq(): boolean {
    // Never reuse a landing node. A native Return can leave a <br> inside the
    // previous span; keeping that DOM would stack a visual line on the image.
    return false;
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
    if (event.type !== "beforeinput") {
      return event.type !== "paste" && event.type !== "compositionstart";
    }
    const inputType = (event as InputEvent).inputType;
    return inputType === "insertText" ||
      inputType === "insertCompositionText" ||
      inputType === "insertLineBreak" ||
      inputType === "insertParagraph";
  }
}

export function landingAnchorsForEmptyLinesAfterImages(
  state: EditorState,
): DecorationSet {
  return Decoration.set(
    emptyLinePositionsAfterImages(state).map((position) =>
      Decoration.widget({
        widget: new MobileEmptyLineCaretAnchorWidget(),
        side: 1,
      }).range(position)
    ),
  );
}

const hideMobileEmptyLineCaretForIme = StateField.define<boolean>({
  create: () => false,
  update(hidden, transaction) {
    let next = hidden;
    for (const effect of transaction.effects) {
      if (effect.is(setHideMobileEmptyLineCaretForIme)) next = effect.value;
    }
    if (
      transaction.docChanged &&
      emptyLinePositionsAfterImages(transaction.state).length === 0
    ) {
      return false;
    }
    return next;
  },
});

const mobileEmptyLineCaretAnchorField = StateField.define<DecorationSet>({
  create: (state) => landingAnchorsForEmptyLinesAfterImages(state),
  update(anchors, transaction) {
    const hidden = transaction.state.field(hideMobileEmptyLineCaretForIme);
    if (hidden) return Decoration.none;
    if (
      transaction.docChanged ||
      transaction.effects.some((effect) =>
        effect.is(setHideMobileEmptyLineCaretForIme)
      )
    ) {
      return landingAnchorsForEmptyLinesAfterImages(transaction.state);
    }
    return anchors;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export function shouldMaterializeMobileEmptyLineBreak(
  inputType: string | undefined,
  insideLandingAnchor: boolean,
): boolean {
  return insideLandingAnchor &&
    (inputType === "insertLineBreak" || inputType === "insertParagraph");
}

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
    "Mobile composer caret kept an image-adjacent landing anchor",
    {
      sequence,
      focus_owned: content.ownerDocument.activeElement === content,
      composing: view.composing,
      state_head: stateSelection.head,
      state_line: view.state.doc.lineAt(stateSelection.head).number,
      document_lines: view.state.doc.lines,
      after_image: selectionOnEmptyLineAfterImage(view.state),
      active_line_empty: activeLine?.textContent === "\u200b",
      anchor_dom_only: true,
      persistent_until_input: false,
      landing_only: true,
      line_top: lineRect ? rounded(lineRect.top - contentRect.top) : -1,
      line_height: lineRect ? rounded(lineRect.height) : -1,
      caret_top: rangeRect ? rounded(rangeRect.top - contentRect.top) : -1,
      caret_height: rangeRect ? rounded(rangeRect.height) : -1,
    },
  );
}

export const mobileEmptyLineCaretRepair = [
  hideMobileEmptyLineCaretForIme,
  mobileEmptyLineCaretAnchorField,
  ViewPlugin.fromClass(
    class {
      syncTimer = 0;
      placeTimer = 0;
      placeFrame = 0;
      pendingBreak = false;
      reportSequence = 0;

      constructor(readonly view: EditorView) {
        if (emptyLinePositionsAfterImages(view.state).length > 0) {
          this.schedulePlace();
        }
      }

      cancelSync(): void {
        if (this.syncTimer !== 0) {
          globalThis.clearTimeout(this.syncTimer);
          this.syncTimer = 0;
        }
      }

      cancelPlace(): void {
        if (this.placeTimer !== 0) {
          globalThis.clearTimeout(this.placeTimer);
          this.placeTimer = 0;
        }
        if (this.placeFrame !== 0) cancelAnimationFrame(this.placeFrame);
        this.placeFrame = 0;
      }

      insideLandingAnchor(target: EventTarget | null): boolean {
        if (!(target instanceof Node)) return false;
        const element = target instanceof Element ? target : target.parentElement;
        return element?.closest(`.${MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS}`) !=
          null;
      }

      setImeHidden(hidden: boolean): void {
        if (this.view.state.field(hideMobileEmptyLineCaretForIme) === hidden) {
          return;
        }
        this.view.dispatch({
          effects: setHideMobileEmptyLineCaretForIme.of(hidden),
        });
      }

      placeLandingSelection(): void {
        if (!selectionOnEmptyLineAfterImage(this.view.state)) return;
        const anchor = this.view.contentDOM.querySelector<HTMLElement>(
          `.cm-line.cm-activeLine .${MOBILE_EMPTY_LINE_CARET_ANCHOR_CLASS}`,
        );
        const text = anchor?.firstChild;
        const selection = this.view.contentDOM.ownerDocument.getSelection();
        if (!text || text.nodeType !== Node.TEXT_NODE || !selection) return;
        // Route the next key into the landing text node. This does not move
        // the UIKit caret by itself — first Return / Backspace still have to
        // be native input inside that node.
        const range = this.view.contentDOM.ownerDocument.createRange();
        range.setStart(text, 0);
        range.collapse(true);
        selection.removeAllRanges();
        selection.addRange(range);
        const sequence = this.reportSequence < 6 ? ++this.reportSequence : 0;
        reportAnchorGeometry(this.view, sequence);
      }

      schedulePlace(): void {
        this.cancelPlace();
        // Never dispatch from update(). The landing widget is already in the
        // document transaction; this only maps the DOM selection into it.
        this.placeTimer = globalThis.setTimeout(() => {
          this.placeTimer = 0;
          this.placeFrame = requestAnimationFrame(() => {
            this.placeFrame = 0;
            this.placeLandingSelection();
          });
        }, 0);
      }

      materializeLineBreak(): void {
        const { state } = this.view;
        const selection = state.selection.main;
        if (!selection.empty || !selectionOnEmptyLineAfterImage(state)) return;
        this.view.dispatch({
          changes: { from: selection.head, insert: "\n" },
          selection: { anchor: selection.head + 1 },
          userEvent: "input",
        });
      }

      update(update: ViewUpdate): void {
        const had = update.startState.field(mobileEmptyLineCaretAnchorField).size;
        const has = update.state.field(mobileEmptyLineCaretAnchorField).size;
        if (update.docChanged) {
          this.pendingBreak = false;
          this.cancelSync();
        }
        if (has > 0 && had === 0) this.schedulePlace();
      }

      destroy(): void {
        this.cancelSync();
        this.cancelPlace();
      }
    },
    {
      eventHandlers: {
        beforeinput(event: InputEvent): void {
          if (event.inputType === "insertCompositionText") {
            this.setImeHidden(true);
            return;
          }
          if (
            event.inputType === "insertText" &&
            event.data &&
            this.insideLandingAnchor(event.target)
          ) {
            event.preventDefault();
            const head = this.view.state.selection.main.head;
            this.view.dispatch({
              changes: { from: head, insert: event.data },
              selection: { anchor: head + event.data.length },
              userEvent: "input.type",
            });
            return;
          }
          if (
            !shouldMaterializeMobileEmptyLineBreak(
              event.inputType,
              this.insideLandingAnchor(event.target),
            )
          ) {
            return;
          }
          // Do not preventDefault: UIKit only moves the caret for a native
          // Return. Never dispatch from update(). Materialize the CM6 line
          // after the key instead.
          this.pendingBreak = true;
          this.cancelSync();
          this.syncTimer = globalThis.setTimeout(() => {
            this.syncTimer = 0;
            if (!this.pendingBreak) return;
            this.pendingBreak = false;
            this.materializeLineBreak();
          }, 0);
        },
        compositionstart(): void {
          this.setImeHidden(true);
        },
        compositionend(): void {
          this.setImeHidden(false);
        },
      },
    },
  ),
];
