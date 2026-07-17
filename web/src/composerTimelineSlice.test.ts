import { assert, assertEquals } from "jsr:@std/assert";
import {
  composerTimelineSlice,
  sameComposerTimelineSlice,
} from "./composerTimelineSlice";
import type { Envelope } from "./protocol";

const update = (seq: number, sessionUpdate: string, extra = {}): Envelope => ({
  session_id: "s",
  seq,
  kind: "update",
  update: { sessionUpdate, ...extra },
});

Deno.test("composer timeline slice ignores ordinary transcript churn", () => {
  const timeline: Envelope[] = [
    update(1, "available_commands_update", {
      availableCommands: [{ name: "compact", description: "Compact" }],
    }),
    update(2, "plan", {
      entries: [{ content: "Ship", status: "in_progress" }],
    }),
    {
      session_id: "s",
      seq: 3,
      kind: "permission_request",
      request_id: "permission-1",
      tool_call: { title: "Deploy" },
      options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
    },
  ];
  const before = composerTimelineSlice(timeline);
  const after = composerTimelineSlice([
    ...timeline,
    update(4, "agent_message_chunk", {
      messageId: "message-1",
      content: { type: "text", text: "streaming" },
    }),
    update(5, "agent_thought_chunk", {
      messageId: "thought-1",
      content: { type: "text", text: "thinking" },
    }),
    update(6, "tool_call", { toolCallId: "tool-1", title: "Inspect" }),
  ]);

  assert(sameComposerTimelineSlice(before, after));
  assertEquals(after.plan?.entries[0]?.content, "Ship");
  assertEquals(after.pendingPermission?.requestId, "permission-1");
  assertEquals(after.availableCommands[0]?.name, "compact");
});

Deno.test("composer timeline slice reacts to every composer-owned signal", () => {
  const base: Envelope[] = [
    update(1, "available_commands_update", {
      availableCommands: [{ name: "compact", description: "Compact" }],
    }),
    update(2, "plan", {
      entries: [{ content: "Ship", status: "in_progress" }],
    }),
    {
      session_id: "s",
      seq: 3,
      kind: "permission_request",
      request_id: "permission-1",
      tool_call: { title: "Deploy" },
      options: [],
    },
  ];
  const initial = composerTimelineSlice(base);

  const planChanged = composerTimelineSlice([
    ...base,
    update(4, "plan", { entries: [{ content: "Ship", status: "completed" }] }),
  ]);
  assert(!sameComposerTimelineSlice(initial, planChanged));

  const permissionChanged = composerTimelineSlice([
    ...base,
    {
      session_id: "s",
      seq: 5,
      kind: "permission_resolved",
      request_id: "permission-1",
      option_id: "allow",
    },
  ]);
  assert(!sameComposerTimelineSlice(initial, permissionChanged));

  const commandsChanged = composerTimelineSlice([
    ...base,
    update(6, "available_commands_update", {
      availableCommands: [{ name: "compress", description: "Compress" }],
    }),
  ]);
  assert(!sameComposerTimelineSlice(initial, commandsChanged));

  const compacting = composerTimelineSlice([
    ...base,
    update(7, "user_message_chunk", {
      content: { type: "text", text: "/compact" },
    }),
  ]);
  assert(!sameComposerTimelineSlice(initial, compacting));

  const completed = composerTimelineSlice([
    ...base,
    update(8, "agent_message_chunk", {
      content: { type: "text", text: "*Context compacted.*" },
    }),
  ]);
  assert(!sameComposerTimelineSlice(initial, completed));
  assertEquals(completed.completionSeq, 8);
});
