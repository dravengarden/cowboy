import { test } from "node:test";
import assert from "node:assert/strict";
import { installSteeringPriority } from "./steering.mjs";

function fixture(run) {
  const messages = [];
  class Agent {
    sessions = {
      one: {
        input: {
          push(message) {
            messages.push(message);
          },
        },
      },
    };
    async steer(request) {
      return await run(this.sessions.one.input, request);
    }
  }
  assert.equal(installSteeringPriority(Agent), true);
  return { agent: new Agent(), messages };
}
const request = (priority) => ({
  sessionId: "one",
  _meta: { steering: { priority, idleBehavior: "promptRequired" } },
});

test("next changes only immediate steering; callbacks and ordinary prompts retain upstream priority", async () => {
  const { agent, messages } = fixture(async (input) => {
    for (const priority of ["now", "later", undefined]) {
      input.push({
        type: "user",
        priority,
        uuid: "original",
        message: { content: "hello" },
      });
    }
    return { outcome: "injected" };
  });
  const push = agent.sessions.one.input.push;
  assert.deepEqual(await agent.steer(request("next")), { outcome: "injected" });
  assert.deepEqual(messages.map((message) => message.priority), [
    "next",
    "later",
    undefined,
  ]);
  assert.ok(messages.every((message) => message.uuid === "original"));
  assert.equal(agent.sessions.one.input.push, push);
  await agent.steer(request("now"));
  assert.equal(messages[3].priority, "now");
});

test("failed and overlapping native steering cannot leave an input override behind", async () => {
  let release;
  const { agent, messages } = fixture(async () => {
    await new Promise((resolve) => {
      release = resolve;
    });
    throw new Error("native refusal");
  });
  const push = agent.sessions.one.input.push;
  const first = agent.steer(request("next"));
  await assert.rejects(agent.steer(request("now")), /already pending/);
  release();
  await assert.rejects(first, /native refusal/);
  assert.equal(agent.sessions.one.input.push, push);
  assert.deepEqual(messages, []);
  await assert.rejects(agent.steer(request("invalid")), /Unsupported/);
});

test("unsupported adapters never advertise priority support", () => {
  class Agent {}
  assert.equal(installSteeringPriority(Agent), false);
});
