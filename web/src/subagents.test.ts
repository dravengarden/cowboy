import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { derive, pendingPermissions, type RenderItem } from "./derive";
import type { AcpUpdate, Envelope } from "./protocol";
import {
  requestingSubagent,
  subagentCounts,
  subagentOf,
  subagentPreview,
  subagentStatus,
} from "./subagents";

function snapshot(seq: number, fields: Record<string, unknown>): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: { sessionUpdate: "cowboy_subagent", sessionId: "child", ...fields } as AcpUpdate,
  };
}

function text(seq: number, value: string): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: value } },
  };
}

function subagentRows(items: RenderItem[]): Extract<RenderItem, { kind: "subagent" }>[] {
  return items.filter((item) => item.kind === "subagent");
}

test("a snapshot becomes a bounded subagent model with a status and preview", () => {
  const subagent = subagentOf({
    sessionUpdate: "cowboy_subagent",
    sessionId: "child",
    revision: 7,
    title: "Explore",
    description: "Explore",
    state: "running",
    cancellable: true,
    prompt: "Find the bug\nin parser.rs",
    toolCalls: 2,
    messages: 1,
    activity: [
      { kind: "tool", id: "t1", title: "Read parser.rs", status: "completed", toolKind: "read" },
      { kind: "message", text: "Looking at the parser" },
      { kind: "bogus" },
    ],
  });
  if (!subagent) throw new Error("expected a subagent");
  assertEquals(subagent.title, "Explore");
  assertEquals(subagent.description, null, "a description equal to the title is not repeated");
  assertEquals(subagent.activity.length, 2);
  assertEquals(subagentStatus(subagent), { label: "Working", tone: "working" });
  assertEquals(subagentCounts(subagent), "2 tool calls · 1 message");
  assertEquals(subagentPreview(subagent), "Looking at the parser");
  assertEquals(subagentOf({ sessionUpdate: "cowboy_subagent" }), null);
  assertEquals(subagentOf({ sessionUpdate: "tool_call", sessionId: "x" }), null);
});

test("an ended subagent previews its result and names how it ended", () => {
  const base = { sessionUpdate: "cowboy_subagent", sessionId: "c", state: "idle" };
  const done = subagentOf({ ...base, stopReason: "end_turn", result: "Found it.\nDetails" });
  if (!done) throw new Error("expected a subagent");
  assertEquals(subagentStatus(done).label, "Done");
  assertEquals(subagentPreview(done), "Found it.");
  assertEquals(subagentStatus(subagentOf({ ...base, stopReason: "cancelled" })!).label, "Stopped");
  assertEquals(subagentStatus(subagentOf({ ...base, stopReason: "error" })!).tone, "failed");
  assertEquals(subagentStatus(subagentOf({ ...base, state: "requires_action" })!).tone, "waiting");
  assertEquals(subagentStatus(subagentOf({ ...base, state: "unknown" })!).label, "Ended");
});

test("snapshots of one child are one row at its first position, newest revision wins", () => {
  const timeline: Envelope[] = [
    text(1, "Delegating "),
    snapshot(2, { revision: 10, title: "Explore", state: "running" }),
    text(3, "now."),
    // A live client can hold an intermediate snapshot after a newer canonical
    // row; an older revision never wins.
    snapshot(4, { revision: 30, title: "Explore", state: "idle", stopReason: "end_turn" }),
    snapshot(5, { revision: 20, title: "Explore", state: "running" }),
  ];
  const items = derive(timeline);
  const rows = subagentRows(items);
  assertEquals(rows.length, 1);
  assertEquals(rows[0]?.key, "2");
  assertEquals(rows[0]?.subagent.state, "idle");
  const message = items.find((item) => item.kind === "message");
  if (message?.kind !== "message" || message.chunks[0]?.type !== "text") {
    throw new Error("expected the parent message");
  }
  assertEquals(message.chunks[0].text, "Delegating now.", "a subagent row never splits the reply");
});

test("a re-announced subagent without labels keeps the earlier ones", () => {
  const rows = subagentRows(derive([
    snapshot(1, { revision: 1, title: "Explore", prompt: "Find it", state: "idle" }),
    snapshot(2, { revision: 2, state: "running" }),
  ]));
  assertEquals(rows[0]?.subagent.title, "Explore");
  assertEquals(rows[0]?.subagent.prompt, "Find it");
  assertEquals(rows[0]?.subagent.state, "running");
});

test("a worker exit settles a working subagent and disables its stop", () => {
  const items = derive([
    snapshot(1, { revision: 1, state: "running", cancellable: true }),
    { session_id: "s", seq: 2, kind: "lifecycle", status: "crashed", detail: "boom" },
  ]);
  const row = subagentRows(items)[0];
  assertEquals(row?.subagent.state, "unknown");
  assertEquals(row?.subagent.cancellable, false);
});

test("a subagent's request names it and outlives the parent turn", () => {
  const request = (seq: number, id: string, toolCall: unknown): Envelope => ({
    session_id: "s",
    seq,
    kind: "permission_request",
    request_id: id,
    tool_call: toolCall,
    options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
  });
  const child = { toolCallId: "t", title: "Run tests", _cowboy: { subagent: { sessionId: "child", title: "Reviewer" } } };
  assertEquals(requestingSubagent(child), "Reviewer");
  assertEquals(requestingSubagent({ _cowboy: { subagent: { sessionId: "c", title: null } } }), "Subagent");
  assertEquals(requestingSubagent({ title: "x" }), null);
  const timeline: Envelope[] = [
    request(1, "1", child),
    request(2, "2", { toolCallId: "p", title: "Edit file" }),
    { session_id: "s", seq: 3, kind: "turn_end", stop_reason: "end_turn" },
  ];
  const pending = pendingPermissions(timeline);
  assertEquals(pending.map((item) => [item.requestId, item.subagent]), [["1", "Reviewer"]]);
  const card = derive(timeline).find((item) => item.kind === "permission");
  assertEquals(card?.kind === "permission" ? card.subagent : null, "Reviewer");
});
