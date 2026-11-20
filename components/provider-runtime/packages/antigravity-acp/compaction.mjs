// Context compaction for a Cowboy Antigravity session. `agy` advertises a
// native `/compact` slash command and runs it as an ordinary
// `session/prompt`: it summarizes the history in that turn but reports
// nothing over ACP, so the turn would end without any visible result. The
// proxy recognizes the command, shows it as ACP's typed `compaction_update`
// (only to a worker that advertised the `session.compaction` client
// capability) and restates the session's Cowboy context on the first owner
// prompt after a completed compaction.
import { randomUUID } from "node:crypto";

/** Sessions whose projection the proxy keeps for a restatement. */
export const MAX_SESSIONS = 64;

/** The one slash command that compacts in `agy` 1.5.9. */
const COMMAND = /^\/compact(?:\s|$)/i;

/** Whether the prompt's blocks are text whose content is `agy`'s `/compact`. */
export function isCompactionPrompt(prompt) {
  if (!Array.isArray(prompt) || prompt.length === 0) return false;
  if (prompt.some((block) => block?.type !== "text" || typeof block.text !== "string")) {
    return false;
  }
  return COMMAND.test(
    prompt
      .map((block) => block.text)
      .join("")
      .trim(),
  );
}

/** Whether the prompt starts with a slash command `agy` would expand. */
export function isSlashPrompt(prompt) {
  const first = Array.isArray(prompt) ? prompt[0] : undefined;
  return first?.type === "text" && typeof first.text === "string"
    ? first.text.trimStart().startsWith("/")
    : false;
}

/** Whether the worker's `initialize` advertised typed compaction updates. */
export function supportsCompaction(params) {
  const compaction = params?.clientCapabilities?.session?.compaction;
  return compaction !== null && typeof compaction === "object" && !Array.isArray(compaction);
}

/**
 * The block prepended to the first owner prompt after a compaction: the
 * projection the session started with (routing, environment, repository and
 * extension guidance), so it stays in effect even if the summary dropped it.
 */
export function restatement(projection) {
  return [
    "<cowboy_context_restated>",
    "This conversation was just compacted. Cowboy restates the session context it gave when the",
    "session started; it still applies. The Git state below was captured at that time and may be",
    "stale; check it with the cowboy tools when it matters.",
    projection,
    "</cowboy_context_restated>",
  ].join("\n");
}

/** The settled status of a compaction turn from the reply the worker gets. */
export function compactionStatus(reply) {
  if (reply.error !== undefined) {
    return { status: "failed", error: String(reply.error?.message ?? "the turn failed") };
  }
  if (reply.result?.stopReason === "cancelled") return { status: "cancelled" };
  return { status: "completed" };
}

/** Per-proxy compaction state: capability, projections and pending restatements. */
export class Compaction {
  constructor() {
    /** Whether the worker accepts `compaction_update` (from `initialize`). */
    this.typed = false;
    /** The projection each session was opened with. */
    this.projections = new Map();
    /** Sessions whose next owner prompt restates the projection. */
    this.pending = new Set();
  }

  initialize(params) {
    this.typed = supportsCompaction(params);
  }

  /** A session was opened (new, load, resume) with `projection`. */
  opened(sessionId, projection) {
    if (typeof sessionId !== "string" || typeof projection !== "string") return;
    this.projections.delete(sessionId);
    if (this.projections.size >= MAX_SESSIONS) {
      const oldest = this.projections.keys().next().value;
      this.projections.delete(oldest);
      this.pending.delete(oldest);
    }
    this.projections.set(sessionId, projection);
    // A freshly opened session already has the projection in its system prompt.
    this.pending.delete(sessionId);
  }

  /** A new compaction for a turn, or `undefined` when the prompt is not one. */
  begin(prompt) {
    return isCompactionPrompt(prompt) ? { id: `agy-compact-${randomUUID()}` } : undefined;
  }

  /** The `session/update` notification for a compaction, when the worker accepts it. */
  update(sessionId, compaction, status, error) {
    if (!this.typed) return undefined;
    const update = { sessionUpdate: "compaction_update", compactionId: compaction.id, status };
    if (error !== undefined) update.error = error;
    return { jsonrpc: "2.0", method: "session/update", params: { sessionId, update } };
  }

  /** A compaction turn ended; a completed one schedules the restatement. */
  settled(sessionId, status) {
    if (status === "completed" && this.projections.has(sessionId)) this.pending.add(sessionId);
  }

  /** A restating turn failed: the next owner prompt restates again. */
  retry(sessionId) {
    if (this.projections.has(sessionId)) this.pending.add(sessionId);
  }

  /** The restatement block for this owner prompt, consumed once, or `undefined`. */
  take(sessionId, prompt) {
    if (!this.pending.has(sessionId) || isSlashPrompt(prompt)) return undefined;
    this.pending.delete(sessionId);
    const projection = this.projections.get(sessionId);
    return projection === undefined ? undefined : restatement(projection);
  }
}
