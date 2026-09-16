import { assert, assertEquals } from "jsr:@std/assert";
import {
  canContinueFailedTurn,
  TURN_CONTINUATION_PROMPT,
  turnFailureTelemetry,
} from "./turnRecovery.ts";

const transcript = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("continuing is offered only where a worker survived the turn", () => {
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

Deno.test("the continuation states the one fact the agent cannot know", () => {
  // Cowboy never replays the original prompt — tools already ran
  // (docs/claude-stream-recovery.md) — so the text must say what happened and
  // what is unsafe to assume, not just "continue".
  assert(TURN_CONTINUATION_PROMPT.includes("transport error"));
  assert(TURN_CONTINUATION_PROMPT.includes("not by me"));
  assert(TURN_CONTINUATION_PROMPT.includes("do not redo work"));
  assert(TURN_CONTINUATION_PROMPT.includes("side effects"));
});

Deno.test("the failure is counted once per card, and never quotes the detail", () => {
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

Deno.test("the card knows which session it is offering to continue", () => {
  assert(
    transcript.includes(
      "sessionId={sessionId}\n                      status={status}",
    ),
  );
  assert(
    transcript.includes("const continuable = canContinueFailedTurn(status);"),
  );
});
