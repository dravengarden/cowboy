import {
  type EditorState,
  Prec,
  StateEffect,
  StateField,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  keymap,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import {
  emptyLinePositionsAfterImages,
  selectionOnEmptyLineAfterImage,
  selectionOnEmptyLineInImageChain,
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
 * A block image is not a `.cm-line`. The trailing landing line is a second
 * document line that looks like part of the thumbnail. Put a U+200B on that
 * landing line only while the caret is actually there — including paste —
 * so the first Return has a native text node. Leave the line, drop the
 * widget; keeping it after the caret moved let v1251 stack native <br>
 * tags inside the same landing node (line_height 14→28→42, document_lines
 * still 2). Mapping the DOM selection into a newly appeared landing node
 * only routes the next key; it is not the Return/Backspace animation.
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
  if (!selectionOnEmptyLineInImageChain(state)) return Decoration.none;
  const line = state.doc.lineAt(state.selection.main.head);
  return Decoration.set([
    Decoration.widget({
      widget: new MobileEmptyLineCaretAnchorWidget(),
      side: 1,
    }).range(line.from),
  ]);
}

export function landingLineBreakSpec(state: EditorState): {
  from: number;
  insert: string;
  anchor: number;
} | null {
  if (!state.selection.main.empty || !selectionOnEmptyLineInImageChain(state)) {
    return null;
  }
  const head = state.selection.main.head;
  return { from: head, insert: "\n", anchor: head + 1 };
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
      transaction.selection ||
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
  state: EditorState,
): boolean {
  return (inputType === "insertLineBreak" || inputType === "insertParagraph") &&
    selectionOnEmptyLineInImageChain(state);
}

/** Physical v1255: iOS fires beforeinput, then a later keydown Enter. */
export const MOBILE_LINE_BREAK_ENTER_CONSUME_MS = 500;

const setMobileEnterConsumeUntil = StateEffect.define<number>();

const mobileEnterConsumeUntil = StateField.define<number>({
  create: () => 0,
  update(until, transaction) {
    for (const effect of transaction.effects) {
      if (effect.is(setMobileEnterConsumeUntil)) return effect.value;
    }
    return until;
  },
});

export function consumeUntilAfterMaterializedLineBreak(nowMs: number): number {
  return nowMs + MOBILE_LINE_BREAK_ENTER_CONSUME_MS;
}

export function shouldConsumeTrailingMobileEnter(
  nowMs: number,
  consumeUntilMs: number,
): boolean {
  return consumeUntilMs > 0 && nowMs < consumeUntilMs;
}

function consumeTrailingMobileEnter(view: EditorView): boolean {
  if (
    !shouldConsumeTrailingMobileEnter(
      Date.now(),
      view.state.field(mobileEnterConsumeUntil),
    )
  ) {
    return false;
  }
  reportClientLog(
    "info",
    "mobile_caret_line_break_consumed",
    "Mobile composer consumed the trailing iOS Enter after a materialized line break",
    {
      document_lines: view.state.doc.lines,
      state_line: view.state.doc.lineAt(view.state.selection.main.head).number,
    },
  );
  return true;
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
      image_chain: selectionOnEmptyLineInImageChain(view.state),
      active_line_empty: activeLine?.textContent === "\u200b",
      anchor_dom_only: true,
      persistent_until_input: false,
      landing_only: false,
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
  mobileEnterConsumeUntil,
  Prec.highest(
    keymap.of([{ key: "Enter", run: consumeTrailingMobileEnter }]),
  ),
  ViewPlugin.fromClass(
    class {
      placeTimer = 0;
      placeFrame = 0;
      reportSequence = 0;

      constructor(readonly view: EditorView) {
        if (emptyLinePositionsAfterImages(view.state).length > 0) {
          this.schedulePlace();
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

      setImeHidden(hidden: boolean): void {
        if (this.view.state.field(hideMobileEmptyLineCaretForIme) === hidden) {
          return;
        }
        this.view.dispatch({
          effects: setHideMobileEmptyLineCaretForIme.of(hidden),
        });
      }

      placeLandingSelection(): void {
        if (!selectionOnEmptyLineInImageChain(this.view.state)) return;
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

      materializeLineBreak(): boolean {
        const spec = landingLineBreakSpec(this.view.state);
        if (!spec) return false;
        this.view.dispatch({
          changes: { from: spec.from, insert: spec.insert },
          selection: { anchor: spec.anchor },
          userEvent: "input",
          effects: setMobileEnterConsumeUntil.of(
            consumeUntilAfterMaterializedLineBreak(Date.now()),
          ),
        });
        return true;
      }

      update(update: ViewUpdate): void {
        const has = update.state.field(mobileEmptyLineCaretAnchorField).size;
        if (has > 0 && (update.docChanged || update.selectionSet)) {
          this.schedulePlace();
        }
      }

      destroy(): void {
        this.cancelPlace();
      }
    },
    {
      eventHandlers: {
        beforeinput(event: InputEvent): boolean | void {
          if (event.inputType === "insertCompositionText") {
            this.setImeHidden(true);
            return;
          }
          if (
            event.inputType === "insertText" &&
            event.data &&
            selectionOnEmptyLineInImageChain(this.view.state)
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
              this.view.state,
            )
          ) {
            return;
          }
          // Physical v1253: beforeinput.target is always .cm-content, so
          // event.target cannot decide this. A native Return then inserts
          // <br> into the landing node (line_height 14→28, still 2 lines).
          // Use the CM6 selection and write a real newline. Never dispatch
          // from update().
          // Physical v1255: iOS still fires keydown Enter ~250ms later.
          // That second event must not insert another newline.
          if (!this.materializeLineBreak()) return;
          event.preventDefault();
          return true;
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
