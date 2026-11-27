import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import {
  canContinueFailedTurn,
  isRefusedInput,
  REFUSED_INPUT_HINT,
  TURN_CONTINUATION_PROMPT,
  turnFailureTelemetry,
} from "./turnRecovery.ts";
import { readTranscriptSource } from "./appSource.test.ts";

const transcript = await readTranscriptSource();

test("continuing is offered only where a worker survived the turn", () => {
  // The plugin's `keep_worker_alive` decision reaches the client as "this
  // session is not crashed" — the controller keeps it out of `crashed` on a
  // recoverable turn failure (src/acp.rs).
  assertEquals(canContinueFailedTurn("running"), true);
  assertEquals(canContinueFailedTurn("busy"), true);
  for (
    const dead of ["crashed", "exited", "interrupted", "starting"] as const
  ) {
    assertEquals(canContinueFailedTurn(dead), false, dead);
  }
});

test("the continuation states the one fact the agent cannot know", () => {
  // Cowboy never replays the original prompt — tools already ran
  // (docs/claude-stream-recovery.md) — so the text must say what happened and
  // what is unsafe to assume, not just "continue".
  assert(TURN_CONTINUATION_PROMPT.includes("transport error"));
  assert(TURN_CONTINUATION_PROMPT.includes("not by me"));
  assert(TURN_CONTINUATION_PROMPT.includes("do not redo work"));
  assert(TURN_CONTINUATION_PROMPT.includes("side effects"));
});

test("the failure is counted once per card, and never quotes the detail", () => {
  // The raw detail can quote file contents from the interrupted turn.
  assertEquals(turnFailureTelemetry("claude-code", "server_error"), {
    provider: "claude-code",
    error_kind: "server_error",
  });
  assertEquals(turnFailureTelemetry("codex", null).error_kind, "unclassified");
  const card = transcript.slice(
    transcript.indexOf("function TurnFailureRecovery"),
    transcript.indexOf("const reportedTurnFailures"),
  );
  assert(
    card.includes(
      'reportClientLog(\n      "warn",\n      "turn_failure_surfaced"',
    ),
  );
  assert(card.includes("reportedTurnFailures.has(itemKey)"));
  assert(card.includes("submitPrompt(sessionId, TURN_CONTINUATION_PROMPT)"));
  // The detail reaches telemetry only through turnFailureTelemetry, which
  // returns the classified kind — never the raw text.
  assert(
    card.includes(
      "turnFailureTelemetry(provider, detail ? rpcErrorKind(detail) : null)",
    ),
  );
});

test("the card knows which session it is offering to continue", () => {
  assert(
    transcript.includes(
      "sessionId={sessionId}\n                      status={status}",
    ),
  );
  assert(
    transcript.includes(
      "const continuable = !refusedInput && canContinueFailedTurn(status);",
    ),
  );
});

test("refused input asks for a text answer, never offers the continuation", () => {
  // sess-1795343291454: images sent while Antigravity awaited an answer.
  const proven = "antigravity_prompt_rejected:image_answer: " +
    '{"cowboy": {"antigravity_backend": {"class": "not_dispatched"}}}';
  assert(isRefusedInput(proven));
  // The bare prefix is not the proxy's result.
  assertEquals(isRefusedInput("antigravity_prompt_rejected:image_answer"), false);
  assertEquals(
    isRefusedInput("antigravity_backend_refusal:rate_limited: {}"),
    false,
  );
  assertEquals(isRefusedInput("worker exited"), false);
  assertEquals(isRefusedInput(null), false);
  // Whether a Draft exists is the Controller's outcome, not the card's.
  assertEquals(REFUSED_INPUT_HINT.includes("Drafts"), false);
  assert(REFUSED_INPUT_HINT.includes("in text"));
  // The card hides Continue for refused input: its text would answer the
  // agent's pending question.
  assert(transcript.includes("const refusedInput = isRefusedInput(detail);"));
  // A remote worker's refusal projects as `crashed`, which hides the failed
  // turn's row behind the status bar; the bar carries the hint then.
  assert(
    transcript.includes(
      'status === "crashed" && isRefusedInput(crashDetail) && (',
    ),
  );
});
