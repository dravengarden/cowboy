// Desktop CM6 ghost text for the agent's predicted next prompt
// (`promptSuggestions.ts`). It is drawn only in an empty document, in place of
// the placeholder, and Tab copies it into the editor without sending. The
// suggestion arrives as a state effect, so a new one never reconfigures the
// editor (no extension churn under Vim or an IME composition).
import {
  type EditorState,
  type Extension,
  StateEffect,
  StateField,
} from "@codemirror/state";
import { Decoration, EditorView, WidgetType } from "@codemirror/view";

export const setPromptSuggestion = StateEffect.define<string | null>();

const suggestionField = StateField.define<string | null>({
  create: () => null,
  update(value, transaction) {
    let next = value;
    for (const effect of transaction.effects) {
      if (effect.is(setPromptSuggestion)) next = effect.value;
    }
    // Typing moves past the prediction; emptying the editor again does not
    // bring it back.
    if (
      next !== null && transaction.docChanged && transaction.newDoc.length > 0
    ) {
      return null;
    }
    return next;
  },
});

function visibleSuggestion(state: EditorState): string | null {
  const suggestion = state.field(suggestionField, false) ?? null;
  return suggestion !== null && state.doc.length === 0 ? suggestion : null;
}

class SuggestionWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }

  override eq(other: SuggestionWidget): boolean {
    return other.text === this.text;
  }

  toDOM(): HTMLElement {
    const hint = document.createElement("span");
    hint.className = "cm-prompt-suggestion";
    hint.dataset["promptSuggestion"] = "ghost";
    hint.setAttribute("aria-hidden", "true");
    hint.textContent = this.text;
    const key = document.createElement("span");
    key.className = "cm-prompt-suggestion-key";
    key.textContent = "Tab";
    hint.append(key);
    return hint;
  }

  override ignoreEvent(): boolean {
    return false;
  }
}

/** Copy the visible suggestion into the empty editor, caret at its end. */
export function acceptPromptSuggestion(view: EditorView): boolean {
  const suggestion = visibleSuggestion(view.state);
  if (suggestion === null) return false;
  view.dispatch({
    changes: { from: 0, insert: suggestion },
    selection: { anchor: suggestion.length },
    effects: setPromptSuggestion.of(null),
    userEvent: "input.complete",
    scrollIntoView: true,
  });
  return true;
}

export function promptSuggestionVisible(state: EditorState): boolean {
  return visibleSuggestion(state) !== null;
}

/** The field, its ghost decoration, and `onDismiss` whenever the held
 *  suggestion ends inside the editor (typed past or accepted). */
export function promptSuggestionExtension(onDismiss: () => void): Extension {
  return [
    suggestionField,
    EditorView.decorations.compute([suggestionField, "doc"], (state) => {
      const suggestion = visibleSuggestion(state);
      return suggestion === null ? Decoration.none : Decoration.set([
        Decoration.widget({ widget: new SuggestionWidget(suggestion), side: 1 })
          .range(0),
      ]);
    }),
    EditorView.editorAttributes.compute(
      [suggestionField, "doc"],
      (state): Record<string, string> =>
        visibleSuggestion(state) === null
          ? {}
          : { class: "cm-has-prompt-suggestion" },
    ),
    EditorView.updateListener.of((update) => {
      const before = update.startState.field(suggestionField, false) ?? null;
      const after = update.state.field(suggestionField, false) ?? null;
      if (before !== null && after === null && update.docChanged) onDismiss();
    }),
    EditorView.baseTheme({
      "&.cm-has-prompt-suggestion .cm-placeholder": { display: "none" },
      ".cm-prompt-suggestion": {
        opacity: "0.5",
        pointerEvents: "none",
        whiteSpace: "pre-wrap",
      },
      ".cm-prompt-suggestion-key": {
        marginLeft: "0.6em",
        padding: "0 0.35em",
        border: "1px solid currentColor",
        borderRadius: "3px",
        fontSize: "0.75em",
        verticalAlign: "1px",
      },
    }),
  ];
}
