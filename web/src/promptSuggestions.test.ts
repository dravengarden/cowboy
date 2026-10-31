import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { EditorState } from "@codemirror/state";
import {
  dismissPromptSuggestion,
  observePromptSuggestion,
  promptSuggestion,
  promptSuggestionOf,
} from "./promptSuggestions";
import {
  promptSuggestionExtension,
  promptSuggestionVisible,
  setPromptSuggestion,
} from "./composer/promptSuggestionExtension";
import type { Envelope } from "./protocol";

// claude-agent-acp 0.99.1 dist/prompt-suggestions.js `promptSuggestionsMeta`.
function info(session: string, seq: number, suggestions: unknown): Envelope {
  return {
    session_id: session,
    seq,
    kind: "update",
    update: {
      sessionUpdate: "session_info_update",
      _meta: { promptSuggestions: { suggestions } },
    },
  };
}

test("prompt suggestion updates parse the first non-blank text and an empty list clears", () => {
  const parse = (update: Record<string, unknown>) =>
    promptSuggestionOf({ sessionUpdate: "session_info_update", ...update });
  assertEquals(
    parse({
      _meta: {
        promptSuggestions: {
          suggestions: [{ text: " " }, { text: "Run the\n tests" }],
        },
      },
    }),
    "Run the tests",
  );
  assertEquals(
    parse({ _meta: { promptSuggestions: { suggestions: [] } } }),
    null,
  );
  // An update that says nothing about suggestions leaves the held one alone.
  assertEquals(parse({ title: "Renamed" }), undefined);
  assertEquals(parse({ _meta: { other: true } }), undefined);
  assertEquals(
    promptSuggestionOf({
      sessionUpdate: "agent_message_chunk",
      _meta: { promptSuggestions: { suggestions: [{ text: "x" }] } },
    }),
    undefined,
  );
});

test("a held suggestion is replaced, cleared by the adapter, a new turn, or a dismissal", () => {
  observePromptSuggestion(info("a", 1, [{ text: "Commit it" }]));
  observePromptSuggestion(info("b", 2, [{ text: "Other session" }]));
  assertEquals(promptSuggestion("a"), "Commit it");
  observePromptSuggestion(info("a", 3, [{ text: "Push it" }]));
  assertEquals(promptSuggestion("a"), "Push it");
  observePromptSuggestion({
    session_id: "a",
    seq: 4,
    kind: "update",
    update: { sessionUpdate: "session_info_update", title: "t" },
  });
  assertEquals(promptSuggestion("a"), "Push it");
  observePromptSuggestion(info("a", 5, []));
  assertEquals(promptSuggestion("a"), null);
  observePromptSuggestion(info("a", 6, [{ text: "Again" }]));
  observePromptSuggestion({
    session_id: "a",
    seq: 7,
    kind: "update",
    update: {
      sessionUpdate: "user_message_chunk",
      content: { type: "text", text: "next" },
    },
  });
  assertEquals(promptSuggestion("a"), null);
  assertEquals(promptSuggestion("b"), "Other session");
  dismissPromptSuggestion("b");
  assertEquals(promptSuggestion("b"), null);
});

test("the editor ghost shows only in an empty document and typing ends it", () => {
  let state = EditorState.create({
    extensions: [promptSuggestionExtension(() => undefined)],
  });
  assertEquals(promptSuggestionVisible(state), false);
  state =
    state.update({ effects: setPromptSuggestion.of("Run the tests") }).state;
  assertEquals(promptSuggestionVisible(state), true);
  state = state.update({ changes: { from: 0, insert: "x" } }).state;
  assertEquals(promptSuggestionVisible(state), false);
  // Emptying the editor again does not bring the prediction back.
  state = state.update({ changes: { from: 0, to: 1 } }).state;
  assertEquals(promptSuggestionVisible(state), false);
  state = state.update({ effects: setPromptSuggestion.of("Later") }).state;
  assertEquals(promptSuggestionVisible(state), true);
  state = state.update({ effects: setPromptSuggestion.of(null) }).state;
  assertEquals(promptSuggestionVisible(state), false);
});
