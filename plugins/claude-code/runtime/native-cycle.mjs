// Claude resumes on its own after a background task ends. ACP runs that as
// foreground work with no prompt turn, so a steer finds no running turn and
// Cowboy can only wait for the cycle to end. Name each such work once, tell
// the worker, and let a steer addressed to exactly that name join it: ACP
// already lets a new turn take a self-started cycle over. Delete this adapter
// when upstream steers a self-started cycle itself.
import { randomUUID } from "node:crypto";

export const NATIVE_CYCLE_SUBTYPE = "cowboy_native_cycle";

const NO_RUNNING_TURN = { outcome: "promptRequired", reason: "noRunningTurn" };

function announce(agent, sessionId, cycleId) {
  // The worker reads this beside the SDK's own session-state frames; an
  // older worker ignores the unknown subtype.
  void Promise.resolve(
    agent.client?.extNotification?.("_claude/sdkMessage", {
      sessionId,
      message: {
        type: "system",
        subtype: NATIVE_CYCLE_SUBTYPE,
        session_id: sessionId,
        cycle_id: cycleId,
      },
    }),
  ).catch(() => {});
}

/** The name of the self-started work that runs now, if any. A turn that took
 *  the work over keeps its name: the work is still the same cycle. */
export function currentNativeCycle(agent, sessionId) {
  return agent.foregroundWork?.get(sessionId)?.cowboyCycleId;
}

export function installNativeCycles(Agent) {
  const proto = Agent.prototype;
  const { startForeground, endForeground, steer, startTurn } = proto;
  if (
    [startForeground, endForeground, steer, startTurn].some(
      (method) => typeof method !== "function",
    ) || typeof proto.runsAutonomousCycle !== "function"
  ) {
    return false;
  }
  proto.startForeground = function (sessionId, owner) {
    const before = this.foregroundWork?.get(sessionId);
    const result = startForeground.call(this, sessionId, owner);
    const work = this.foregroundWork?.get(sessionId);
    if (!before && work && this.runsAutonomousCycle(sessionId)) {
      work.cowboyCycleId = `native-${randomUUID()}`;
      announce(this, sessionId, work.cowboyCycleId);
    }
    return result;
  };
  proto.endForeground = function (sessionId, owner, end) {
    const work = this.foregroundWork?.get(sessionId);
    const result = endForeground.call(this, sessionId, owner, end);
    if (work?.cowboyCycleId && this.foregroundWork?.get(sessionId) !== work) {
      announce(this, sessionId, null);
    }
    return result;
  };
  proto.steer = async function (request) {
    const cycle = request?._meta?.steering?.nativeCycle;
    if (cycle === undefined) return await steer.call(this, request);
    const sessionId = request.sessionId;
    const session = this.sessions?.[sessionId];
    // Checked synchronously with the decision below: a cycle that ended or
    // was replaced is never joined, and the message stays with the caller.
    if (
      typeof cycle !== "string" || !session || session.queryClosed ||
      currentNativeCycle(this, sessionId) !== cycle
    ) {
      return NO_RUNNING_TURN;
    }
    // A steer already joined this cycle: its turn owns the work, so steer
    // into that turn as usual.
    if ((session.turnQueue ?? []).some((turn) => !turn.settled)) {
      return await steer.call(this, request);
    }
    const input = session.input;
    if (!input || typeof input.push !== "function") return NO_RUNNING_TURN;
    // Deliver into the running cycle rather than behind it, as a steer does
    // for a running turn; an open question keeps upstream's `later`.
    const priority = (session.openUserInputs?.size ?? 0) > 0
      ? "later"
      : request._meta.steering.priority === "next"
      ? "next"
      : "now";
    const push = input.push;
    const agent = this;
    let pending = true;
    input.push = function (message) {
      if (pending && message?.type === "user") {
        pending = false;
        input.push = push;
        // `startTurn` awaits before it queues. Should the cycle have ended
        // meanwhile, the message runs as an ordinary next turn and never
        // jumps into a different cycle.
        return push.call(
          this,
          currentNativeCycle(agent, sessionId) === cycle
            ? { ...message, priority }
            : message,
        );
      }
      return push.call(this, message);
    };
    try {
      // The new turn takes the self-started work over; its outcome is the
      // cycle's, which the worker already follows through session state.
      await startTurn.call(
        this,
        { sessionId, prompt: request.prompt },
        { inserted() {}, ended() {}, failed() {} },
      );
    } catch {
      // Refused before it was queued: nothing reached Claude.
      return NO_RUNNING_TURN;
    } finally {
      if (pending) input.push = push;
    }
    return { outcome: "injected" };
  };
  return true;
}
