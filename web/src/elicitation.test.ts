import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { permissionFallbackChoices } from "./ElicitationFields";
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

test("Antigravity ask_question form elicitation parses single, multi, and optional Other fields", () => {
  const elicitation = elicitationOf(
    carrying({
      sessionId: "agy-sess-1",
      toolCallId: "call_2211613383716556954",
      mode: "form",
      message: "The agent has 2 questions.",
      requestedSchema: {
        type: "object",
        properties: {
          q1: {
            type: "string",
            title: "Copy format?",
            oneOf: [
              { const: "1", title: "(Recommended) HTML + plain" },
              { const: "2", title: "Plain only" },
            ],
          },
          q1_other: {
            type: "string",
            title: "Other",
            description: "Optional free-form answer to: Copy format?",
          },
          q2: {
            type: "array",
            title: "Surfaces?",
            items: {
              anyOf: [
                { const: "1", title: "Mobile" },
                { const: "2", title: "Desktop" },
              ],
            },
          },
          q2_other: {
            type: "string",
            title: "Other",
            description: "Optional free-form answer to: Surfaces?",
          },
        },
      },
    }),
  );
  if (elicitation?.mode !== "form") throw new Error("expected a form");
  assertEquals(
    elicitation.fields.map((field) => [field.id, field.kind, field.required]),
    [
      ["q1", "single", false],
      ["q1_other", "text", false],
      ["q2", "multi", false],
      ["q2_other", "text", false],
    ],
  );
  const values = initialElicitationValues(elicitation.fields);
  if (!elicitationComplete(elicitation.fields, values)) {
    throw new Error("optional fields allow submitting write-in or partial answers");
  }
  values["q1"] = "1";
  values["q2"] = ["1", "2"];
  values["q2_other"] = "prioritize iOS";
  assertEquals(
    elicitationAnswer(elicitation.fields, values),
    'accept:{"q1":"1","q2":["1","2"],"q2_other":"prioritize iOS"}',
  );
});

test("parent tool completion with a colliding toolCallId keeps child form pending in both derive() and pendingPermissions() (child last)", () => {
  const timeline: Envelope[] = [
    {
      seq: 1,
      ts: 1,
      kind: "permission_request",
      request_id: "parent-req-1",
      tool_call: { toolCallId: "call_1", title: "Parent tool" },
      options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
    },
    {
      seq: 2,
      ts: 2,
      kind: "permission_request",
      request_id: "child-req-1",
      tool_call: {
        toolCallId: "call_1",
        title: "Subagent question",
        _cowboy: {
          subagent: { sessionId: "child-1", title: "Helper" },
          elicitation: {
            mode: "form",
            sessionId: "child-1",
            toolCallId: "call_1",
            message: "Subagent question",
            requestedSchema: { type: "object", properties: { q1: { type: "string", title: "Q" } } },
          },
        },
      },
      options: [{ optionId: "accept", name: "Submit", kind: "allow_once" }],
    },
    {
      seq: 3,
      ts: 3,
      kind: "update",
      update: { sessionUpdate: "tool_call_update", toolCallId: "call_1", status: "completed" },
    },
  ];
  const pending = pendingPermissions(timeline);
  assertEquals(pending.map((p) => p.requestId), ["child-req-1"]);
  assertEquals(pending[0]?.subagentId, "child-1");

  const derived = derive(timeline).filter((item) => item.kind === "permission");
  assertEquals(
    derived.map((item) => [item.requestId, item.resolved]),
    [
      ["parent-req-1", true],
      ["child-req-1", false],
    ],
  );
});

test("elicitation bounds, verbatim choices, Unicode code-point lengths, and __proto__ keys stay aligned with proxy", () => {
  const schemaWithBounds = JSON.parse(
    JSON.stringify({
      mode: "form",
      sessionId: "s",
      message: "Bounded form",
      requestedSchema: {
        type: "object",
        properties: {
          score: { type: "integer", title: "Score", minimum: 1, maximum: 5 },
          choice: {
            type: "string",
            title: "Choice",
            oneOf: [
              { const: " spaced ", title: "Spaced" },
              { const: "too-long", title: "Too long" },
            ],
            maxLength: 8,
          },
          emoji: { type: "string", title: "Emoji", maxLength: 1 },
        },
        required: ["score", "choice"],
      },
    }),
  );
  const form = elicitationOf(carrying(schemaWithBounds));
  if (form?.mode !== "form") throw new Error("expected form");
  // Out-of-range integer blocks Submit.
  assertEquals(elicitationComplete(form.fields, { score: "6", choice: " spaced ", emoji: "" }), false);
  assertEquals(elicitationComplete(form.fields, { score: "0", choice: " spaced ", emoji: "" }), false);
  // Single emoji is 1 Unicode code point (UTF-16 length 2) and satisfies maxLength: 1.
  assertEquals(elicitationComplete(form.fields, { score: "3", choice: " spaced ", emoji: "😀" }), true);
  assertEquals(elicitationComplete(form.fields, { score: "3", choice: " spaced ", emoji: "😀😁" }), false);
  // Choice with surrounding whitespace is serialized verbatim (never trimmed).
  assertEquals(
    elicitationAnswer(form.fields, { score: "3", choice: " spaced ", emoji: "😀" }),
    'accept:{"score":3,"choice":" spaced ","emoji":"😀"}',
  );

  // Impossible required multi-select (minItems > distinct choices) is unrenderable.
  assertEquals(
    elicitationOf(
      carrying({
        mode: "form",
        sessionId: "s",
        message: "Impossible",
        requestedSchema: {
          type: "object",
          properties: {
            tags: {
              type: "array",
              items: { enum: ["a", "b"] },
              minItems: 3,
            },
          },
          required: ["tags"],
        },
      }),
    ),
    null,
  );

  // Own "__proto__" property survives initialElicitationValues, elicitationComplete, and elicitationAnswer.
  const protoForm = elicitationOf(
    carrying({
      mode: "form",
      sessionId: "s",
      message: "Proto field",
      requestedSchema: {
        type: "object",
        properties: JSON.parse('{"__proto__":{"type":"string","title":"Proto"}}'),
        required: ["__proto__"],
      },
    }),
  );
  if (protoForm?.mode !== "form") throw new Error("expected proto form");
  const values = initialElicitationValues(protoForm.fields);
  Object.defineProperty(values, "__proto__", {
    value: "kept",
    enumerable: true,
    writable: true,
    configurable: true,
  });
  assertEquals(elicitationComplete(protoForm.fields, values), true);
  assertEquals(elicitationAnswer(protoForm.fields, values), 'accept:{"__proto__":"kept"}');
});

test("terminal tool update clears every pending form sharing that scoped toolCallId (R3)", () => {
  const timeline: Envelope[] = [
    {
      seq: 1,
      ts: 1,
      kind: "permission_request",
      request_id: "req-1",
      tool_call: { toolCallId: "shared_tool", title: "First ask" },
      options: [{ optionId: "accept", name: "Submit", kind: "allow_once" }],
    },
    {
      seq: 2,
      ts: 2,
      kind: "permission_request",
      request_id: "req-2",
      tool_call: { toolCallId: "shared_tool", title: "Second ask" },
      options: [{ optionId: "accept", name: "Submit", kind: "allow_once" }],
    },
    {
      seq: 3,
      ts: 3,
      kind: "update",
      update: { sessionUpdate: "tool_call_update", toolCallId: "shared_tool", status: "completed" },
    },
  ];
  assertEquals(pendingPermissions(timeline), []);
  const derived = derive(timeline).filter((item) => item.kind === "permission");
  assertEquals(
    derived.map((item) => [item.requestId, item.resolved]),
    [
      ["req-1", true],
      ["req-2", true],
    ],
  );
});

test("invalid forms expose only Decline, empty-string choices require explicit selection, and bounded optional choices never trap Submit (R3)", () => {
  // 1. Optional impossible integer range [1.2, 1.8] makes the whole form invalid -> only Decline.
  const impossibleToolCall = carrying({
    mode: "form",
    sessionId: "s",
    message: "Impossible optional int",
    requestedSchema: {
      type: "object",
      properties: {
        badInt: { type: "integer", title: "Bad", minimum: 1.2, maximum: 1.8 },
        ok: { type: "string", title: "OK" },
      },
    },
  });
  assertEquals(elicitationOf(impossibleToolCall), null);
  const pending = pendingPermissions([
    {
      seq: 1,
      ts: 1,
      kind: "permission_request",
      request_id: "bad-form",
      tool_call: impossibleToolCall,
      options: [
        { optionId: "accept", name: "Submit", kind: "allow_once" },
        { optionId: "decline", name: "Decline", kind: "reject_once" },
      ],
    },
  ]);
  assertEquals(pending.length, 1);
  assertEquals(pending[0]?.invalidElicitation, true);
  assertEquals(pending[0]?.requestKind, "question");
  const picked: string[] = [];
  const fallback = permissionFallbackChoices(
    pending[0]!.options,
    pending[0]!.invalidElicitation,
    (id) => picked.push(id),
  );
  assertEquals(
    fallback.map((c) => [c.id, c.reject]),
    [["decline", true]],
  );
  fallback[0]!.run();
  assertEquals(picked, ["decline"]);

  // 2. Required single-choice offering "" starts UNSET and does not enable Submit until selected.
  const emptyChoiceForm = elicitationOf(
    carrying({
      mode: "form",
      sessionId: "s",
      message: "Empty choice",
      requestedSchema: {
        type: "object",
        properties: {
          q1: {
            type: "string",
            title: "Pick",
            oneOf: [
              { const: "", title: "None" },
              { const: "a", title: "A" },
            ],
          },
        },
        required: ["q1"],
      },
    }),
  );
  if (emptyChoiceForm?.mode !== "form") throw new Error("expected form");
  const initVals = initialElicitationValues(emptyChoiceForm.fields);
  assertEquals(Object.hasOwn(initVals, "q1"), false);
  assertEquals(elicitationComplete(emptyChoiceForm.fields, initVals), false);
  assertEquals(elicitationComplete(emptyChoiceForm.fields, { q1: "" }), true);
  assertEquals(elicitationAnswer(emptyChoiceForm.fields, { q1: "" }), 'accept:{"q1":""}');

  // 3. Optional bounded single-choice filters invalid choices, ignores invalid default, and omits when none fit.
  const boundedOpt = elicitationOf(
    carrying({
      mode: "form",
      sessionId: "s",
      message: "Optional bounded choices",
      requestedSchema: {
        type: "object",
        properties: {
          partial: {
            type: "string",
            title: "Partial",
            oneOf: [
              { const: "ok", title: "OK" },
              { const: "too-long", title: "Too long" },
            ],
            maxLength: 2,
            default: "too-long",
          },
          allInvalid: {
            type: "string",
            title: "None fit",
            oneOf: [{ const: "too-long", title: "Too long" }],
            maxLength: 2,
          },
        },
      },
    }),
  );
  if (boundedOpt?.mode !== "form") throw new Error("expected form");
  assertEquals(
    boundedOpt.fields.map((f) => [f.id, f.choices.map((c) => c.value), f.defaultValue]),
    [["partial", ["ok"], undefined]],
  );
  const optVals = initialElicitationValues(boundedOpt.fields);
  assertEquals(elicitationComplete(boundedOpt.fields, optVals), true);
});
