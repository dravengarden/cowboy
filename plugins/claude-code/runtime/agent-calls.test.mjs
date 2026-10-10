import assert from "node:assert/strict";
import test from "node:test";
import { AGENT_CALLS_PROMPT } from "./agent-calls.mjs";

test("the agent-calls prompt routes reviews through Cowboy", () => {
  assert.match(AGENT_CALLS_PROMPT, /cowboy codex --request-file -/);
  assert.match(AGENT_CALLS_PROMPT, /instead of running `codex exec`/);
  // A Claude agent never calls its own family.
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /cowboy claude/);
  // One short paragraph: it rides every turn's system prompt.
  assert.ok(AGENT_CALLS_PROMPT.length < 1200);
  const example = AGENT_CALLS_PROMPT.match(/\{"schema".*\}\}/)[0]
    .replace("review|design_review|analysis", "review");
  assert.equal(JSON.parse(example).access, "read-only");
});
