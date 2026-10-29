import { test } from "node:test";
import assert from "node:assert/strict";
import { currentNativeCycle, installNativeCycles } from "./native-cycle.mjs";

const AUTONOMOUS = "autonomous-cycle";

// The parts of ACP's agent this adapter extends, with upstream's contract:
// new foreground work starts a cycle, a turn takes running work over, and
// only the owner's end deletes it.
function fixture({ startTurnDelay = null } = {}) {
  const notices = [];
  const pushed = [];
  const steers = [];
  class Agent {
    client = {
      extNotification: async (method, params) =>
        notices.push({ method, params }),
    };
    foregroundWork = new Map();
    sessions = {
      one: {
        turnQueue: [],
        input: { push: (message) => pushed.push(message) },
      },
    };
    startForeground(sessionId, owner) {
      const current = this.foregroundWork.get(sessionId);
      if (current) {
        current.owner = owner;
        return;
      }
      this.foregroundWork.set(sessionId, { owner, awaitingUser: false });
    }
    endForeground(sessionId, owner) {
      if (this.foregroundWork.get(sessionId)?.owner !== owner) return;
      this.foregroundWork.delete(sessionId);
    }
    runsAutonomousCycle(sessionId) {
      return this.foregroundWork.get(sessionId)?.owner === AUTONOMOUS;
    }
    async steer(request) {
      steers.push(request);
      return { outcome: "injected" };
    }
    async startTurn(params) {
      if (startTurnDelay) await startTurnDelay(this);
      const session = this.sessions[params.sessionId];
      const turn = { settled: false };
      session.turnQueue.push(turn);
      session.input.push({
        type: "user",
        uuid: "prompt",
        message: params.prompt,
      });
      this.startForeground(params.sessionId, turn);
    }
  }
  assert.equal(installNativeCycles(Agent), true);
  const cycles = () =>
    notices.map(({ method, params }) => {
      assert.equal(method, "_claude/sdkMessage");
      assert.equal(params.message.subtype, "cowboy_native_cycle");
      assert.equal(params.message.session_id, params.sessionId);
      return params.message.cycle_id;
    });
  return { agent: new Agent(), pushed, steers, cycles };
}

const steerTo = (cycle, priority = "now") => ({
  sessionId: "one",
  prompt: [{ type: "text", text: "correction" }],
  _meta: {
    steering: { priority, idleBehavior: "promptRequired", nativeCycle: cycle },
  },
});

test("only a self-started cycle gets a name, announced at its start and end", () => {
  const { agent, cycles } = fixture();
  agent.startForeground("one", { turn: true });
  assert.equal(
    currentNativeCycle(agent, "one"),
    undefined,
    "a prompt turn is not a cycle",
  );
  agent.endForeground("one", agent.foregroundWork.get("one").owner);
  agent.startForeground("one", AUTONOMOUS);
  const first = currentNativeCycle(agent, "one");
  assert.match(first, /^native-/);
  agent.startForeground("one", AUTONOMOUS);
  assert.equal(
    currentNativeCycle(agent, "one"),
    first,
    "the same work keeps its name",
  );
  agent.endForeground("one", AUTONOMOUS);
  agent.startForeground("one", AUTONOMOUS);
  const second = currentNativeCycle(agent, "one");
  assert.notEqual(second, first, "every cycle has its own name");
  assert.deepEqual(cycles(), [first, null, second]);
});

test("a steer joins exactly the named cycle as a new turn with its priority", async () => {
  const { agent, pushed, steers, cycles } = fixture();
  agent.startForeground("one", AUTONOMOUS);
  const cycle = currentNativeCycle(agent, "one");
  const push = agent.sessions.one.input.push;
  assert.deepEqual(await agent.steer(steerTo(cycle, "next")), {
    outcome: "injected",
  });
  assert.equal(pushed.length, 1);
  assert.equal(pushed[0].priority, "next");
  assert.equal(
    agent.sessions.one.input.push,
    push,
    "the push override is gone",
  );
  assert.equal(steers.length, 0, "the upstream refusal path is not used");
  // The turn took the work over: same cycle, and a second steer goes into
  // that turn through the ordinary path.
  assert.equal(currentNativeCycle(agent, "one"), cycle);
  assert.deepEqual(await agent.steer(steerTo(cycle)), { outcome: "injected" });
  assert.equal(steers.length, 1);
  assert.equal(pushed.length, 1);
  // The turn's end ends the cycle.
  agent.endForeground("one", agent.foregroundWork.get("one").owner);
  assert.deepEqual(cycles(), [cycle, null]);
});

test("a stale, foreign or ended cycle is refused without reaching Claude", async () => {
  const { agent, pushed, steers } = fixture();
  const refused = { outcome: "promptRequired", reason: "noRunningTurn" };
  assert.deepEqual(await agent.steer(steerTo("native-none")), refused);
  agent.startForeground("one", AUTONOMOUS);
  const cycle = currentNativeCycle(agent, "one");
  assert.deepEqual(await agent.steer(steerTo("native-other")), refused);
  assert.deepEqual(await agent.steer(steerTo(42)), refused);
  agent.endForeground("one", AUTONOMOUS);
  assert.deepEqual(await agent.steer(steerTo(cycle)), refused);
  agent.sessions.one.queryClosed = true;
  agent.startForeground("one", AUTONOMOUS);
  assert.deepEqual(
    await agent.steer(steerTo(currentNativeCycle(agent, "one"))),
    refused,
  );
  assert.deepEqual(pushed, []);
  assert.deepEqual(steers, []);
});

test("an open question keeps upstream's later priority", async () => {
  const { agent, pushed } = fixture();
  agent.sessions.one.openUserInputs = new Set(["question"]);
  agent.startForeground("one", AUTONOMOUS);
  await agent.steer(steerTo(currentNativeCycle(agent, "one")));
  assert.equal(pushed[0].priority, "later");
});

test("a cycle that ends while the turn is admitted never receives the message", async () => {
  let swap;
  const { agent, pushed } = fixture({
    startTurnDelay: async (self) => {
      swap(self);
      await Promise.resolve();
    },
  });
  swap = (self) => {
    self.endForeground("one", AUTONOMOUS);
    self.startForeground("one", AUTONOMOUS);
  };
  agent.startForeground("one", AUTONOMOUS);
  const cycle = currentNativeCycle(agent, "one");
  assert.deepEqual(await agent.steer(steerTo(cycle)), { outcome: "injected" });
  assert.notEqual(currentNativeCycle(agent, "one"), cycle);
  assert.equal(pushed.length, 1);
  assert.equal(
    pushed[0].priority,
    undefined,
    "queued behind, not into, the new cycle",
  );
});

test("a refusal before the turn is queued restores the input and delivers nothing", async () => {
  const { agent, pushed } = fixture({
    startTurnDelay: async () => {
      throw new Error("signed out");
    },
  });
  agent.startForeground("one", AUTONOMOUS);
  const push = agent.sessions.one.input.push;
  assert.deepEqual(
    await agent.steer(steerTo(currentNativeCycle(agent, "one"))),
    { outcome: "promptRequired", reason: "noRunningTurn" },
  );
  assert.equal(agent.sessions.one.input.push, push);
  assert.deepEqual(pushed, []);
});

test("ordinary steering is untouched and old adapters are left alone", async () => {
  const { agent, steers } = fixture();
  const ordinary = {
    sessionId: "one",
    _meta: { steering: { priority: "now" } },
  };
  assert.deepEqual(await agent.steer(ordinary), { outcome: "injected" });
  assert.equal(steers[0], ordinary);
  class Old {
    steer() {}
  }
  assert.equal(installNativeCycles(Old), false);
});
