// ACP 0.99 owns active-turn admission, UUID tracking and settlement across SDK
// result cycles. Extend only its SDK input priority; never duplicate that turn
// lifecycle. Delete this adapter when upstream accepts the priority itself.

// Claude Code aborts the running generation for a `now` message, and while it
// is compacting that generation is the summary request: the steer fails the
// compaction. Downgrade to `next` until the compaction reaches its terminal
// status so the message joins the turn after the summary lands. Delete with
// the adapter above once upstream defers immediate steering itself.
export function compactionInProgress(session) {
  const active = session?.contextCompaction?.activeCompaction;
  return active !== undefined && !active.terminalStatus;
}

export function installSteeringPriority(Agent) {
  const steer = Agent.prototype.steer;
  if (typeof steer !== "function") return false;
  const active = new WeakSet();
  Agent.prototype.steer = async function (request) {
    const requested = request?._meta?.steering?.priority ?? "now";
    if (requested !== "now" && requested !== "next") {
      throw new Error("Unsupported steering priority");
    }
    const session = this.sessions?.[request.sessionId];
    const priority = requested === "now" && compactionInProgress(session) ? "next" : requested;
    const input = session?.input;
    if (input && active.has(input)) {
      throw new Error("Steering is already pending");
    }
    if (priority === "now") return await steer.call(this, request);
    if (
      session?.needsSignOutRespawn &&
      (session.queryClosed || !(session.turnQueue ?? []).some((turn) => !turn.settled))
    ) {
      throw new Error("The captured steering session is unavailable");
    }
    if (!input || typeof input.push !== "function") {
      throw new Error("Steering input is unavailable");
    }
    const push = input.push;
    active.add(input);
    input.push = function (message) {
      // Keep upstream's permission/elicitation protection (`later`) intact.
      // Ordinary prompts have no priority and cannot acquire this override.
      return push.call(
        this,
        message?.type === "user" && message.priority === "now"
          ? { ...message, priority: "next" }
          : message,
      );
    };
    try {
      return await steer.call(this, request);
    } finally {
      input.push = push;
      active.delete(input);
    }
  };
  return true;
}
