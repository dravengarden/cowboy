import { expect, test } from "bun:test";
import { derive, forkableAnswerKeys } from "./derive";
import type { Envelope } from "./protocol";

let seq = 0;
function update(
  sessionUpdate: string,
  fields: Record<string, unknown> = {},
): Envelope {
  seq += 1;
  return {
    session_id: "s1",
    seq,
    kind: "update",
    update: {
      sessionUpdate,
      content: { type: "text", text: `t${String(seq)}` },
      ...fields,
    },
  } as Envelope;
}

test("a fork opens with its origin and offers only turn-final answers", () => {
  seq = 0;
  const items = derive([
    update("cowboy_fork_origin", {
      sourceSessionId: "source",
      sourceTitle: "Source",
      messageId: "m0",
    }),
    update("user_message_chunk"),
    update("agent_message_chunk", { messageId: "a1" }),
    update("tool_call", { toolCallId: "t1", title: "Read", status: "completed" }),
    update("agent_message_chunk", { messageId: "a2" }),
    update("agent_message_chunk", { messageId: "a2" }),
    update("user_message_chunk"),
    update("agent_message_chunk", { messageId: "b1" }),
  ]);
  expect(items[0]).toEqual({
    kind: "forked",
    key: "1",
    sourceSessionId: "source",
    sourceTitle: "Source",
    atMessage: true,
  });
  const answers = items.filter((item) =>
    item.kind === "message" && item.role === "assistant"
  );
  expect(answers.map((item) => item.kind === "message" && item.messageId))
    .toEqual(["a1", "a2", "b1"]);

  const idle = forkableAnswerKeys(items, false);
  expect([...idle.values()]).toEqual(["a2", "b1"]);
  // The running turn's answer is not a fork point yet.
  expect([...forkableAnswerKeys(items, true).values()]).toEqual(["a2"]);
});

test("answers without a Provider message ID are never fork points", () => {
  seq = 0;
  const items = derive([
    update("user_message_chunk"),
    update("agent_message_chunk"),
  ]);
  expect(forkableAnswerKeys(items, false).size).toBe(0);
});
