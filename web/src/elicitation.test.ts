import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  elicitationAnswer,
  elicitationComplete,
  elicitationOf,
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
  });
  assertEquals(elicitation.fields[1]?.noteFor, "database");

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
