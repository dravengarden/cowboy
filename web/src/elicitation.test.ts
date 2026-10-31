import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  autoResolutionSecondsLeft,
  elicitationAnswer,
  elicitationComplete,
  elicitationOf,
  formatCountdown,
  initialElicitationValues,
} from "./elicitation";
import { derive, pendingPermissions } from "./derive";
import type { Envelope } from "./protocol";

function carrying(elicitation: Record<string, unknown>): unknown {
  return { title: elicitation["message"], _cowboy: { elicitation } };
}

// The form Codex ACP builds for `request_user_input`: an option question with
// an "Other" choice and its companion note field.
const CODEX_FORM = {
  mode: "form",
  sessionId: "s",
  toolCallId: "item-1",
  message: "Codex needs your input to continue.",
  requestedSchema: {
    type: "object",
    properties: {
      database: {
        type: "string",
        title: "Which database should the service use?",
        description: "Storage",
        oneOf: [
          { const: "Postgres", title: "Postgres", description: "Recommended" },
          { const: "SQLite", title: "SQLite" },
          { const: "Other", title: "Other" },
        ],
        _meta: { codex: { isOther: true, isSecret: false } },
      },
      database_note: {
        type: "string",
        title: "Additional answer or note",
        _meta: { codex: { questionId: "database", role: "user_note", isSecret: false } },
      },
      token: {
        type: "string",
        title: "API token",
        _meta: { codex: { isSecret: true } },
      },
    },
    required: ["database", "token"],
  },
};

test("a Codex user-input form becomes typed fields", () => {
  const elicitation = elicitationOf(carrying(CODEX_FORM));
  if (elicitation?.mode !== "form") throw new Error("expected a form");
  assertEquals(elicitation.message, "Codex needs your input to continue.");
  assertEquals(
    elicitation.fields.map((field) => [field.id, field.kind, field.required, field.secret]),
    [
      ["database", "single", true, false],
      ["database_note", "text", false, false],
      ["token", "text", true, true],
    ],
  );
  assertEquals(elicitation.fields[0]?.choices[0], {
    value: "Postgres",
    title: "Postgres",
    description: "Recommended",
    preview: null,
  });
  assertEquals(elicitation.fields[1]?.noteFor, "database");
  assertEquals(elicitation.autoResolutionMs, null);

  const values = initialElicitationValues(elicitation.fields);
  if (elicitationComplete(elicitation.fields, values)) {
    throw new Error("required answers are missing");
  }
  values["database"] = "SQLite";
  values["token"] = "secret";
  if (!elicitationComplete(elicitation.fields, values)) {
    throw new Error("required answers are present");
  }
  // The empty optional note is omitted.
  assertEquals(
    elicitationAnswer(elicitation.fields, values),
    'accept:{"database":"SQLite","token":"secret"}',
  );
});

test("Claude's AskUserQuestion multi-select and typed answers serialize as the schema expects", () => {
  const elicitation = elicitationOf(carrying({
    mode: "form",
    sessionId: "s",
    message: "Please answer the following questions.",
    requestedSchema: {
      type: "object",
      properties: {
        question_0: {
          type: "array",
          title: "Features",
          items: { anyOf: [{ const: "Auth", title: "Auth" }, { const: "Search", title: "Search" }] },
        },
        retries: { type: "integer", title: "Retries", default: 3 },
        verbose: { type: "boolean", title: "Verbose" },
      },
    },
  }));
  if (elicitation?.mode !== "form") throw new Error("expected a form");
  assertEquals(elicitation.fields.map((field) => field.kind), ["multi", "integer", "boolean"]);
  const values = initialElicitationValues(elicitation.fields);
  assertEquals(values, { question_0: [], retries: "3", verbose: false });
  values["question_0"] = ["Search"];
  assertEquals(
    elicitationAnswer(elicitation.fields, values),
    'accept:{"question_0":["Search"],"retries":3,"verbose":false}',
  );
  values["retries"] = "1.5";
  if (elicitationComplete(elicitation.fields, values)) {
    throw new Error("a malformed integer must not submit");
  }
});

test("a URL elicitation needs an http link; unknown required fields are unanswerable", () => {
  assertEquals(
    elicitationOf(carrying({
      mode: "url",
      requestId: 1,
      elicitationId: "e",
      url: "https://example.com/oauth",
      message: "Sign in",
    })),
    { mode: "url", message: "Sign in", url: "https://example.com/oauth" },
  );
  assertEquals(
    elicitationOf(carrying({
      mode: "url",
      requestId: 1,
      elicitationId: "e",
      url: "javascript:alert(1)",
      message: "Sign in",
    })),
    null,
  );
  assertEquals(
    elicitationOf(carrying({
      mode: "form",
      sessionId: "s",
      message: "m",
      requestedSchema: {
        type: "object",
        properties: { blob: { type: "object" } },
        required: ["blob"],
      },
    })),
    null,
  );
  assertEquals(elicitationOf({ title: "Run ls?", rawInput: { command: "ls" } }), null);
});

test("an elicitation permission request is a question that carries its form", () => {
  const timeline: Envelope[] = [{
    session_id: "s",
    seq: 1,
    kind: "permission_request",
    request_id: "7",
    tool_call: carrying(CODEX_FORM),
    options: [
      { optionId: "accept", name: "Submit", kind: "allow_once" },
      { optionId: "decline", name: "Decline", kind: "reject_once" },
    ],
  }];
  const pending = pendingPermissions(timeline);
  assertEquals(pending.length, 1);
  assertEquals(pending[0]?.requestKind, "question");
  assertEquals(pending[0]?.elicitation?.mode, "form");
  const marker = derive(timeline)[0];
  if (marker?.kind !== "permission") throw new Error("expected a permission marker");
  assertEquals(marker.requestKind, "question");
});

test("AskUserQuestion option previews parse as plain text for single and multi choices", () => {
  const meta = (preview: unknown) => ({ _meta: { "_claude/askUserQuestionOption": { preview } } });
  const elicitation = elicitationOf(carrying({
    mode: "form",
    sessionId: "s",
    message: "Which layout?",
    requestedSchema: {
      type: "object",
      properties: {
        question_0: {
          type: "string",
          oneOf: [
            { const: "Grid", title: "Grid", ...meta("<div>grid</div>\n  cells") },
            { const: "List", title: "List", ...meta("  ") },
            { const: "Rows", title: "Rows", ...meta(7) },
          ],
        },
        question_1: {
          type: "array",
          items: { anyOf: [{ const: "Dark", title: "Dark", ...meta("bg: #000") }] },
        },
      },
    },
  }));
  if (elicitation?.mode !== "form") throw new Error("expected a form");
  assertEquals(
    elicitation.fields[0]?.choices.map((choice) => choice.preview),
    ["<div>grid</div>\n  cells", null, null],
  );
  assertEquals(elicitation.fields[1]?.choices[0]?.preview, "bg: #000");
  const huge = elicitationOf(carrying({
    mode: "form",
    message: "x",
    requestedSchema: { type: "object", properties: {
      q: { type: "string", oneOf: [{ const: "A", ...meta("a".repeat(30_000)) }] },
    } },
  }));
  if (huge?.mode !== "form") throw new Error("expected a form");
  const preview = huge.fields[0]?.choices[0]?.preview ?? "";
  if (preview.length > 20_010 || !preview.endsWith("…")) throw new Error("preview must stay bounded");
});

test("Codex auto-resolution parses and counts down from first sight", () => {
  const timed = elicitationOf(carrying({ ...CODEX_FORM, _meta: { codex: { autoResolutionMs: 90_000 } } }));
  if (timed?.mode !== "form") throw new Error("expected a form");
  assertEquals(timed.autoResolutionMs, 90_000);
  for (const value of [null, "60000", -1, Number.NaN]) {
    const untimed = elicitationOf(carrying({ ...CODEX_FORM, _meta: { codex: { autoResolutionMs: value } } }));
    if (untimed?.mode !== "form") throw new Error("expected a form");
    assertEquals(untimed.autoResolutionMs, null);
  }
  assertEquals(autoResolutionSecondsLeft(90_000, 1_000, 1_000), 90);
  assertEquals(autoResolutionSecondsLeft(90_000, 1_000, 30_500), 61);
  assertEquals(autoResolutionSecondsLeft(90_000, 1_000, 200_000), 0);
  assertEquals(formatCountdown(42), "42s");
  assertEquals(formatCountdown(65), "1:05");
});
