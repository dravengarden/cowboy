import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import { latestAvailableCommands } from "./agentCommands";
import { derive, latestPendingPermission, latestPlan, linkTimeline } from "./derive";
import type { Envelope } from "./protocol";
import { retainTimelineState } from "./timelineRetention";

const update = (seq: number, sessionUpdate: string, extra = {}): Envelope => ({
  session_id: "s",
  seq,
  kind: "update",
  update: { sessionUpdate, ...extra },
});

Deno.test("retention preserves stateful UI checkpoints outside the render tail", () => {
  const timeline: Envelope[] = [
    update(1, "available_commands_update", {
      availableCommands: [{ name: "compact", description: "Compact" }],
    }),
    update(2, "plan", { entries: [{ content: "Ship", status: "in_progress" }] }),
    {
      session_id: "s",
      seq: 3,
      kind: "permission_request",
      request_id: "p",
      tool_call: { title: "Deploy" },
      options: [],
    },
    ...Array.from({ length: 8 }, (_, index) => update(index + 4, "agent_message_chunk")),
  ];
  const retained = retainTimelineState(timeline, 3);

  assertEquals(retained.recentStartSeq, 9);
  assertEquals(latestPlan(retained.events)?.entries[0]?.content, "Ship");
  assertEquals(latestAvailableCommands(retained.events)[0]?.name, "compact");
  assertEquals(latestPendingPermission(retained.events)?.requestId, "p");
  assertEquals(retained.events.slice(-3).map((event) => event.seq), [9, 10, 11]);
});

Deno.test("retention keeps the marker that supersedes an older plan", () => {
  const timeline: Envelope[] = [
    update(1, "plan", { entries: [{ content: "Old", status: "completed" }] }),
    update(2, "user_message_chunk"),
    ...Array.from({ length: 5 }, (_, index) => update(index + 3, "agent_message_chunk")),
  ];

  const plan = latestPlan(retainTimelineState(timeline, 2).events);
  assertEquals(plan?.supersededByUserTurn, true);
});

Deno.test("retention shares unchanged render rows across a history trim", () => {
  const timeline = Array.from({ length: 8 }, (_, index) =>
    update(index + 1, index % 2 === 0 ? "user_message_chunk" : "agent_message_chunk", {
      content: { type: "text", text: `message ${index}` },
    })
  );
  const before = derive(timeline);
  const retained = retainTimelineState(timeline, 4);
  const after = derive(linkTimeline(retained.events, timeline));

  assertEquals(after.map((item) => item.key), ["5", "6", "7", "8"]);
  for (const item of after) {
    assertStrictEquals(item, before.find((candidate) => candidate.key === item.key));
  }
});
