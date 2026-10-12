import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
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

test("composer timeline slice ignores ordinary transcript churn", () => {
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

test("composer timeline slice reacts to every composer-owned signal", () => {
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

test("context clear is a composer input-interaction boundary", () => {
  const before = composerTimelineSlice([
    update(1, "available_commands_update"),
  ]);
  const after = composerTimelineSlice([
    update(1, "available_commands_update"),
    update(2, "context_cleared"),
  ]);
  assertEquals(before.contextClearedSeq, 0);
  assertEquals(after.contextClearedSeq, 2);
  assert(!sameComposerTimelineSlice(before, after));
});

test("pending permissions are answered oldest first and counted", () => {
  const request = (seq: number, id: string): Envelope => ({
    session_id: "s",
    seq,
    kind: "permission_request",
    request_id: id,
    tool_call: { title: id },
    options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
  });
  const waiting: Envelope[] = [request(1, "first"), request(2, "second")];
  const both = composerTimelineSlice(waiting);
  assertEquals(both.pendingPermission?.requestId, "first");
  assertEquals(both.pendingPermissionCount, 2);

  const one = composerTimelineSlice([
    ...waiting,
    {
      session_id: "s",
      seq: 3,
      kind: "permission_resolved",
      request_id: "first",
      option_id: "allow",
    },
  ]);
  assertEquals(one.pendingPermission?.requestId, "second");
  assertEquals(one.pendingPermissionCount, 1);
  assert(!sameComposerTimelineSlice(both, one));
});

test("a request whose resolution never arrived stops waiting", () => {
  const request = (seq: number, id: string, tool: string): Envelope => ({
    session_id: "s",
    seq,
    kind: "permission_request",
    request_id: id,
    tool_call: { title: tool, toolCallId: tool },
    options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
  });
  const count = (timeline: Envelope[]): number =>
    composerTimelineSlice(timeline).pendingPermissionCount;

  // Its tool call finished: it was answered, the row was just lost.
  const ran: Envelope[] = [
    request(1, "1", "tool-a"),
    update(2, "tool_call_update", { toolCallId: "tool-a", status: "completed" }),
  ];
  assertEquals(count(ran), 0);

  // Progress on the tool is not an answer.
  assertEquals(
    count([
      request(1, "1", "tool-a"),
      update(2, "tool_call_update", {
        toolCallId: "tool-a",
        status: "in_progress",
      }),
    ]),
    1,
  );

  // The id is asked again by a new agent process: one request, the new one.
  const reused = composerTimelineSlice([
    request(1, "1", "tool-a"),
    request(2, "2", "tool-b"),
    request(3, "1", "tool-c"),
  ]);
  assertEquals(reused.pendingPermissionCount, 2);
  assertEquals(reused.pendingPermission?.title, "tool-b");

  // The turn ended, or the worker stopped: nobody is waiting.
  assertEquals(
    count([
      request(1, "1", "tool-a"),
      { session_id: "s", seq: 2, kind: "turn_end", stop_reason: "end_turn" },
    ]),
    0,
  );
  assertEquals(
    count([
      request(1, "1", "tool-a"),
      { session_id: "s", seq: 2, kind: "lifecycle", status: "exited", detail: null },
    ]),
    0,
  );
  assertEquals(
    count([
      request(1, "1", "tool-a"),
      { session_id: "s", seq: 2, kind: "lifecycle", status: "busy", detail: null },
    ]),
    1,
  );
});
