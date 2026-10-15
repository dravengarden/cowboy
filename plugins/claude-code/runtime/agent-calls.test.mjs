import assert from "node:assert/strict";
import test from "node:test";
import { AGENT_CALLS_PROMPT } from "./agent-calls.mjs";

test("the agent-calls prompt routes reviews through Cowboy", () => {
  assert.match(AGENT_CALLS_PROMPT, /`cowboy review`/);
  assert.match(AGENT_CALLS_PROMPT, /Never run `codex exec`/);
  // A Claude agent never calls its own family.
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /cowboy claude/);
  // A pointer, not a manual: it rides every turn's system prompt.
  assert.ok(AGENT_CALLS_PROMPT.length < 900, AGENT_CALLS_PROMPT.length);
  // The agent decides, and skipping is the default.
  assert.match(AGENT_CALLS_PROMPT, /Skip it by default/);
  assert.match(AGENT_CALLS_PROMPT, /at most once per task/);
  assert.match(AGENT_CALLS_PROMPT, /say nothing/);
  assert.match(AGENT_CALLS_PROMPT, /cowboy review --ask/);
  // No capability dump or JSON template on every turn.
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /capabilities|"schema"/);
});
