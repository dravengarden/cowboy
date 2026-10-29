// ACP 0.99 owns active-turn admission, UUID tracking and settlement across SDK
// result cycles. Extend only its SDK input priority; never duplicate that turn
// lifecycle. Delete this adapter when upstream accepts the priority itself.
export function installSteeringPriority(Agent) {
  const steer = Agent.prototype.steer;
  if (typeof steer !== "function") return false;
  const active = new WeakSet();
  Agent.prototype.steer = async function (request) {
    const priority = request?._meta?.steering?.priority ?? "now";
    if (priority !== "now" && priority !== "next") {
      throw new Error("Unsupported steering priority");
    }
    const session = this.sessions?.[request.sessionId];
    const input = session?.input;
    if (input && active.has(input)) {
      throw new Error("Steering is already pending");
    }
    if (priority === "now") return await steer.call(this, request);
    if (
      session?.needsSignOutRespawn && (session.queryClosed ||
        !(session.turnQueue ?? []).some((turn) => !turn.settled))
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
