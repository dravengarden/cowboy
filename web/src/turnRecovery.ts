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

/** The proxy's closed prefix of a prompt the agent refused before running it
 *  (Antigravity: images while its `ask_question` awaits an answer), and the
 *  evidence class it always carries. Nothing ran. */
const REFUSED_INPUT_PREFIX = "antigravity_prompt_rejected:";
const REFUSED_INPUT_EVIDENCE = '"not_dispatched"';

/** What the reader does next after refused input: only a text reply unblocks
 *  the agent. The card makes no claim about Drafts; when the Controller
 *  restored the message, the Drafts list shows it. */
export const REFUSED_INPUT_HINT =
  "Answer the agent's question in text first; it takes your next message as " +
  "the answer, so Cowboy offers no Continue here.";

/** Whether a failed turn's detail is input the agent refused before running
 *  it: the proxy's prefix with its `not_dispatched` evidence, as the
 *  Provider's error rule matches it. The transport continuation must never be
 *  offered there: the agent would read it as the answer to its question. */
export function isRefusedInput(detail: string | null): boolean {
  return detail != null && detail.startsWith(REFUSED_INPUT_PREFIX) &&
    detail.includes(REFUSED_INPUT_EVIDENCE);
}
