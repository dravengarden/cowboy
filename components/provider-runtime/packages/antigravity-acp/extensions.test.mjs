import assert from "node:assert/strict";
import test from "node:test";

import { Extensions } from "./extensions.mjs";
import { UncertainEffect } from "./facade-tools.mjs";
import {
  bridgedFetch,
  callGuidance,
  openMemory,
  promptText,
  updateObservation,
} from "./memory.mjs";

const remoteError = (error) => Object.assign(new Error("remote"), { remote: error });

test("the worker's tools become cowboy tools and calls go through it", async () => {
  const calls = [];
  const connection = {
    async call(method, params) {
      calls.push([method, params]);
      if (method === "cowboy/extensions") {
        return {
          tools: [
            { name: "cowboy_read", server: "cowboy", tool: "cowboy_read" },
            { name: "suger_query", server: "suger", tool: "query", description: "q" },
            { name: "bad name", server: "x" },
          ],
          instructions: "cowboy: Prefer Cowboy MCP.",
        };
      }
      if (params.name === "lost") throw remoteError({ data: { outcome: "unknown" } });
      if (params.name === "refused") throw remoteError({ message: "no Cowboy turn is running" });
      return { content: [{ type: "text", text: params.name }] };
    },
  };
  const extensions = await Extensions.open(connection);
  assert.deepEqual(
    extensions.tools().map((tool) => tool.name),
    ["cowboy_read", "suger_query"],
  );
  assert.deepEqual(extensions.route("suger_query"), { server: "suger", tool: "query" });
  assert.equal(extensions.instructions(), "cowboy: Prefer Cowboy MCP.");
  assert.deepEqual(await extensions.call(connection, "suger_query", { a: 1 }), {
    content: [{ type: "text", text: "suger_query" }],
  });
  assert.deepEqual(calls.at(-1), [
    "cowboy/extensionCall",
    { name: "suger_query", arguments: { a: 1 } },
  ]);
  await assert.rejects(extensions.call(connection, "lost", {}), UncertainEffect);
  const refused = await extensions.call(connection, "refused", {});
  assert.equal(refused.isError, true);
  assert.match(refused.content[0].text, /no Cowboy turn/);
});

test("a required binding that failed refuses the session", async () => {
  const connection = {
    async call() {
      throw remoteError({ message: "suger is unavailable: exited" });
    },
  };
  await assert.rejects(Extensions.open(connection));
});

test("the memory client's requests reach the worker without a credential", async () => {
  const seen = [];
  const connection = {
    async call(method, params) {
      seen.push([method, params]);
      return {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"text":"recalled"}',
      };
    },
  };
  const response = await bridgedFetch(connection)("http://cowboy-memory.invalid/v1/context", {
    method: "POST",
    headers: { Authorization: "Bearer bridged", "Content-Type": "application/json" },
    body: "{}",
  });
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { text: "recalled" });
  assert.deepEqual(seen[0], [
    "cowboy/memory",
    {
      path: "/v1/context",
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    },
  ]);
  assert.equal(await openMemory(connection, { memory: null }, { home: "/h", cwd: "/w" }), null);
  // An outbox needs a valid session key.
  assert.equal(
    await openMemory(
      connection,
      { session: "../x", memory: { enable_recall: true } },
      { home: "/h", cwd: "/w" },
    ),
    null,
  );
});

test("installed call guidance is rendered for this session", () => {
  assert.equal(
    callGuidance({
      calls: {
        profiles: [
          { guidance: "Use cowboy {{peer_command}} with {{peer}}." },
          { guidance: "Two." },
        ],
      },
    }),
    "Use cowboy codex with Codex.\n\nTwo.",
  );
  assert.equal(callGuidance({ calls: { profiles: [] } }), "");
});

test("prompts and updates feed the turn's memory evidence", () => {
  assert.equal(
    promptText([
      { type: "text", text: "a" },
      { type: "image", data: "" },
      { type: "text", text: "b" },
    ]),
    "a\nb",
  );
  assert.deepEqual(
    updateObservation({
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: "hi" },
    }),
    ["assistant", "hi"],
  );
  const tool = updateObservation({
    sessionUpdate: "tool_call_update",
    status: "completed",
    title: "Read a",
    content: [{ type: "content", content: { type: "text", text: "body" } }],
  });
  assert.deepEqual(JSON.parse(tool[1]), { title: "Read a", status: "completed", output: "body" });
  const raw = updateObservation({
    sessionUpdate: "tool_call_update",
    status: "completed",
    rawOutput: { n: 1 },
  });
  assert.equal(JSON.parse(raw[1]).output, '{"n":1}');
  assert.equal(updateObservation({ sessionUpdate: "tool_call", status: "pending" }), null);
});
