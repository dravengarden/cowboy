// Recovering the turn after a transport failure.
//
// A Provider whose response stream dies mid-answer leaves a specific state: the
// ACP worker and its native session are alive (the plugin's error rule said
// `keep_worker_alive`, which the controller transports by keeping the session
// out of `crashed`), the work already done is real, and the conversation can be
// continued by sending one more message. Cowboy deliberately does not resend the
// original prompt — tools have already run, and a replay could run them twice
// (docs/claude-stream-recovery.md).
//
// So the recovery is a CONTINUATION, and until now the reader had to type it by
// hand every time. This module owns the two things that were implicit in that
// hand-typed "continue": what the agent is told, and when offering it is honest.

import type { Status } from "./protocol";

/**
 * What the reader would have typed, except it also states the one fact the
 * agent cannot know: the interruption was the transport, not a decision — and
 * therefore what is unsafe to assume about half-finished work.
 */
export const TURN_CONTINUATION_PROMPT =
  "The previous response was cut off by a transport error, not by me. " +
  "Pick up where it stopped: do not redo work that already completed, and " +
  "before re-running any command with side effects, check whether it already " +
  "took effect.";

/**
 * Whether continuing is a real offer. A failed turn only leaves a continuable
 * session when the worker survived it; `crashed`/`exited` sessions need a
 * restart, and offering a button that silently queues a message into a dead
 * session would be a lie.
 */
export function canContinueFailedTurn(status: Status): boolean {
  // `running` is the idle-but-alive state a kept-alive worker lands in, and
  // `busy` means a turn is already in flight (the continuation queues behind
  // it, which is what the reader wants). `starting` is still booting, and
  // `exited`/`crashed`/`interrupted` have no worker to continue.
  return status === "running" || status === "busy";
}

/** Fields for the one telemetry event this surface emits. The point is to turn
 *  "it happens now and then" into a number: which provider, which error class,
 *  how often. Keep it to the classified shape — never the raw detail, which can
 *  quote file contents from the interrupted turn. */
export function turnFailureTelemetry(
  provider: string,
  errorKind: string | null,
): Record<string, string> {
  return {
    provider,
    error_kind: errorKind ?? "unclassified",
  };
}
