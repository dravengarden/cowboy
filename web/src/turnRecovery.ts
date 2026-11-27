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
 *  (Antigravity: images while its `ask_question` awaits an answer). */
const REFUSED_INPUT_PREFIX = "antigravity_prompt_rejected:";
/** The codes the Controller recognises (`PROMPT_REJECTION_CODES`). */
const REFUSED_INPUT_CODES: ReadonlySet<string> = new Set(["image_answer"]);

/** What the reader does next after refused input: only a text reply unblocks
 *  the agent, which takes the next message as its answer; then Back to Prompt
 *  returns the refused message, images included (refusedMessage.ts). */
export const REFUSED_INPUT_HINT =
  "Answer the agent's question in text first, then use Back to Prompt to " +
  "send this message again.";

function plainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Whether a failed turn's detail is input the agent refused before running
 *  it: the proxy's complete, self-consistent result, checked as the
 *  Controller checks it (`account_controls::is_prompt_rejection`): the code
 *  repeated in `cowboy.prompt_rejected`, no violation or possible effect, and
 *  `not_dispatched` evidence with its barrier, one prompt receipt and no
 *  dispatch, success, retry or refusal. The transport continuation must never
 *  be offered there: the agent would read it as the answer to its question. */
export function isRefusedInput(detail: string | null): boolean {
  if (detail == null || !detail.startsWith(REFUSED_INPUT_PREFIX)) return false;
  const split = detail.indexOf(": ");
  if (split < 0) return false;
  const code = detail.slice(REFUSED_INPUT_PREFIX.length, split);
  let data: unknown;
  try {
    data = JSON.parse(detail.slice(split + 2));
  } catch {
    return false;
  }
  if (!plainObject(data) || !plainObject(data.cowboy)) return false;
  const cowboy = data.cowboy;
  const backend = cowboy.antigravity_backend;
  return REFUSED_INPUT_CODES.has(code) && cowboy.prompt_rejected === code &&
    cowboy.policy_violation === undefined &&
    cowboy.uncertain_effect === undefined &&
    plainObject(backend) && backend.class === "not_dispatched" &&
    backend.barrier === true && backend.prompts === 1 &&
    backend.dispatches === 0 && backend.successes === 0 &&
    backend.transient === 0 && Array.isArray(backend.refusals) &&
    backend.refusals.length === 0;
}
