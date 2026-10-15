import assert from "node:assert/strict";
import test from "node:test";
import { AGENT_CALLS_PROMPT, agentCallsPrompt } from "./agent-calls.mjs";

test("the agent-calls prompt routes reviews through Cowboy", () => {
  assert.match(AGENT_CALLS_PROMPT, /cowboy codex --request-file -/);
  assert.match(AGENT_CALLS_PROMPT, /instead of running `codex exec`/);
  // A Claude agent never calls its own family.
  assert.doesNotMatch(AGENT_CALLS_PROMPT, /cowboy claude/);
  // One short paragraph: it rides every turn's system prompt.
  assert.ok(AGENT_CALLS_PROMPT.length < 1800);
  // Optional: off means silence, on means review before reporting.
  assert.match(
    AGENT_CALLS_PROMPT,
    /If they are off, do not review and do not mention it/,
  );
  const example = AGENT_CALLS_PROMPT.match(/\{"schema".*\}\}/)[0]
    .replace("review|design_review|analysis", "review");
  assert.equal(JSON.parse(example).access, "read-only");
});

test("installed call profiles replace the built-in guidance for Claude", () => {
  const guidance =
    'To have another agent review or analyze work (for example when asked to call {{peer}}), use Cowboy instead of running `codex exec`, `codex review` or `claude -p` yourself. Run `cowboy {{peer_command}} --request-file - [--preset ID]` with this JSON on stdin: {"schema":1,"request_id":"<new unique id>","purpose":"review|design_review|analysis","instruction":"<self-contained task>","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}. It prints a JSON envelope with a call_id; then run `cowboy call wait <call_id>` until it is terminal and `cowboy call result <call_id>`. `cowboy call capabilities` reports whether calls are enabled and lists the preset ids (model and reasoning); omit --preset to use the session\'s choice. Reviews are optional and the user turns them on per session: when you have finished a substantive code change and are about to report, run `cowboy call capabilities`; if calls are enabled and {{peer}} is available, get a read-only review from {{peer}} first and fix or report its findings. If they are off, do not review and do not mention it. When the user explicitly asks for a call while calls are off, Cowboy asks them and the command waits for the answer; report a refusal instead of working around it.';
  const prompt = agentCallsPrompt({
    schema: 1,
    calls: { profiles: [{ guidance }] },
  });
  assert.match(prompt, /cowboy codex --request-file -/);
  assert.match(prompt, /review from Codex first/);
  assert.doesNotMatch(prompt, /\{\{/);
  assert.equal(agentCallsPrompt(null), AGENT_CALLS_PROMPT);
});
