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
 * so the first Return has a native text node. Keep that landing node after
 * the caret leaves so the line does not collapse to height 0 and reflow.
 * v1251 stacked native <br> tags in an abandoned node; insertLineBreak is
 * now preventDefaulted, so the magnet is gone. Mapping the DOM selection
 * into a newly appeared landing node only routes the next key; it is not
 * the Return/Backspace animation.
 */
class MobileEmptyLineCaretAnchorWidget extends WidgetType {
  constructor(readonly at: number) {
    super();
  }

  override eq(other: MobileEmptyLineCaretAnchorWidget): boolean {
    // Physical v1259: `eq()` true for every instance let CM6 move the
    // landing node onto the new line. The abandoned landing line then
    // collapsed and grew 14px, and the native Range died.
    return other.at === this.at;
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

export function landingAnchorPositions(state: EditorState): number[] {
  // Physical v1260: decorating every landing line mounted a second widget
  // ~1s later (widgets 1→2). That late node gave the abandoned line its
  // 14px and killed the native Range. Only the caret line needs a
  // U+200B. The image-adjacent line keeps height via CSS.
  if (!selectionOnEmptyLineInImageChain(state)) return [];
  return [state.doc.lineAt(state.selection.main.head).from];
}

export function landingAnchorsForEmptyLinesAfterImages(
  state: EditorState,
): DecorationSet {
  const positions = landingAnchorPositions(state);
  if (positions.length === 0) return Decoration.none;
  return Decoration.set(
    positions.map((from) =>
      Decoration.widget({
        widget: new MobileEmptyLineCaretAnchorWidget(from),
        side: 1,
      }).range(from)
    ),
  );
}

export function shouldPreventNativeMobileLineBreak(
  inputType: string | undefined,
  state: EditorState,
): boolean {
  return (inputType === "insertLineBreak" || inputType === "insertParagraph") &&
    selectionOnEmptyLineInImageChain(state);
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

export function landingSelectionAlreadyPlaced(
  selection: { anchorNode: Node | null; isCollapsed: boolean } | null,
  text: Node | null,
): boolean {
  return Boolean(
    text &&
      text.nodeType === 3 &&
      selection?.isCollapsed &&
      selection.anchorNode === text,
  );
}

export function collapseLandingSelection(
  selection: Selection | null,
  text: Node | null,
): boolean {
  if (!text || text.nodeType !== 3 || !selection) return false;
  if (landingSelectionAlreadyPlaced(selection, text)) return false;
  selection.collapse(text, 0);
  return true;
}

export function updateInsertedLineBreak(update: ViewUpdate): boolean {
  if (!update.docChanged) return false;
  let found = false;
  update.changes.iterChanges((_fromA, _toA, _fromB, _toB, inserted) => {
    if (inserted.toString().includes("\n")) found = true;
  });
  return found;
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
        const text = anchor?.firstChild ?? null;
        const selection = this.view.contentDOM.ownerDocument.getSelection();
        if (
          !collapseLandingSelection(selection, text) &&
          !landingSelectionAlreadyPlaced(selection, text)
        ) {
          return;
        }
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

      update(update: ViewUpdate): void {
        const has = update.state.field(mobileEmptyLineCaretAnchorField).size;
        if (has === 0) return;
        // Physical v1259: after Return, CM6 had already updated the DOM.
        // A delayed removeAllRanges remap bounced; skipping it left the
        // native Range dead. Collapse into the new line's own widget in
        // this same update, without removeAllRanges.
        if (updateInsertedLineBreak(update)) {
          this.placeLandingSelection();
          return;
        }
        if (update.docChanged || update.selectionSet) this.schedulePlace();
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
            !shouldPreventNativeMobileLineBreak(
              event.inputType,
              this.view.state,
            )
          ) {
            return;
          }
          // Physical v1253: beforeinput.target is always .cm-content.
          // Prevent the native <br> only. v1255–v1257 wrote `\n` here and
          // then either accepted or consumed the later keydown Enter —
          // one Return became two visual caret moves. Let that single
          // keydown insert the document line.
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
