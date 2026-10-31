import { afterEach, test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  applySubagentAuthority,
  barSummary,
  buildTaskSummaries,
  callSummary,
  groupTasks,
  originToolKey,
  reconcileSubagents,
  sessionAuthority,
  setSubagentLookupTransport,
  type SubagentStateWire,
  subagentSummary,
  taskHash,
  taskKeyFromHash,
} from "./backgroundTasks";
import { callApprovalDecisionBody, type CallApproval } from "./callApproval";
import { mergeCanonicalTimeline } from "./canonicalTimeline";
import { derive, type RenderItem } from "./derive";
import type { ManagedCallSummary } from "./managedCalls";
import type { AcpUpdate, Envelope } from "./protocol";
import { subagentOf, subagentStatus } from "./subagents";

function subagentUpdate(child: string, state: string, revision: number, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    sessionUpdate: "cowboy_subagent",
    sessionId: child,
    revision,
    title: `Agent ${child}`,
    state,
    stopReason: state === "idle" ? "end_turn" : null,
    cancellable: state === "running",
    ...extra,
  };
}

function subagent(seq: number, child: string, state: string, revision: number, extra: Record<string, unknown> = {}): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: subagentUpdate(child, state, revision, extra) as AcpUpdate,
  };
}

function filler(seq: number): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: { sessionUpdate: "agent_message_chunk", messageId: `m${String(seq)}`, content: { type: "text", text: "x" } },
  };
}

function chips(items: RenderItem[]): Record<string, string> {
  return Object.fromEntries(
    items.flatMap((item) => item.kind === "subagent" ? [[item.subagent.id, subagentStatus(item.subagent).label]] : []),
  );
}

function state(items: SubagentStateWire["items"], extra: Partial<SubagentStateWire> = {}): SubagentStateWire {
  return {
    session_id: "s",
    epoch: "boot-1",
    watermark_seq: 1_000,
    complete: true,
    incarnation: "worker-1",
    items,
    ...extra,
  };
}

const lookups: string[][] = [];
afterEach(() => {
  setSubagentLookupTransport(null);
  lookups.length = 0;
});

// T1: the device cached child A as running, went offline, and A finished
// while more than 300 other events arrived. The Controller replaced A's anchor
// row in place, so the reconnect tail (the newest 200 events) no longer
// carries it. Before the fix this rendered "Working" forever (see the
// pre-fix run recorded in docs/background-tasks.md); now the bootstrap's
// subagent_state repairs it.
test("T1 an offline completion beyond the reconnect tail shows Done", async () => {
  const cached = [subagent(10, "a", "running", 1, { observedSeq: 10 }), filler(11)];
  const tail = Array.from({ length: 200 }, (_, index) => filler(301 + index));
  const merged = mergeCanonicalTimeline(cached, tail);
  assertEquals(chips(derive(merged)), { a: "Working" }, "the tail alone cannot repair the row");
  await reconcileSubagents(
    "t1",
    state([{ anchor_seq: 10, update: subagentUpdate("a", "idle", 2, { observedSeq: 450 }) }]),
    merged,
  );
  const authority = sessionAuthority("t1");
  assertEquals(chips(applySubagentAuthority(derive(merged), authority)), { a: "Done" });
  assertEquals(authority?.entries.get("a")?.reconciledBy, "bootstrap");
});

// T1 (lookup path) and I6: a cached non-terminal child the bootstrap did not
// list is resolved by lookup; not_found is "No longer available", never Done.
test("T1/I6 uncovered cached children are looked up; not_found is never finished", async () => {
  setSubagentLookupTransport((_, ids) => {
    lookups.push(ids);
    return Promise.resolve(ids.map((id) =>
      id === "gone"
        ? { id, status: "not_found" as const }
        : { id, status: "found" as const, anchor_seq: 1, update: subagentUpdate(id, "idle", 9, { observedSeq: 99 }) }
    ));
  });
  const timeline = [subagent(1, "a", "running", 1), subagent(2, "gone", "running", 1)];
  const report = await reconcileSubagents("i6", state([]), timeline);
  assertEquals(lookups, [["a", "gone"]]);
  assertEquals(report.notFound, 1);
  const items = applySubagentAuthority(derive(timeline), sessionAuthority("i6"));
  assertEquals(chips(items), { a: "Done", gone: "No longer available" });
  assertEquals(sessionAuthority("i6")?.entries.get("a")?.reconciledBy, "lookup");
});

// T2: more than 50 later completions. There is no newest-N window: every
// cached id is reconciled whatever the number of later completions.
test("T2 sixty later completions do not hide the earlier one", async () => {
  setSubagentLookupTransport((_, ids) => {
    lookups.push(ids);
    return Promise.resolve([]);
  });
  const cached = [subagent(1, "a", "running", 1)];
  const items = [
    { anchor_seq: 1, update: subagentUpdate("a", "idle", 2, { observedSeq: 900 }) },
    ...Array.from({ length: 60 }, (_, index) => ({
      anchor_seq: 100 + index,
      update: subagentUpdate(`later-${String(index)}`, "idle", 2, { observedSeq: 500 + index }),
    })),
  ];
  await reconcileSubagents("t2", state(items), cached);
  const authority = sessionAuthority("t2");
  assertEquals(authority?.entries.size, 61);
  assertEquals(chips(applySubagentAuthority(derive(cached), authority)), { a: "Done" });
  assertEquals(lookups, [], "everything was covered by the bootstrap");
  const listed = buildTaskSummaries({
    subagents: [...(authority?.entries.values() ?? [])].map((entry) => ({ ...entry, subagent: entry.subagent })),
    calls: [],
    approval: null,
    pending: [],
  });
  assertEquals(listed.filter((task) => task.chip === "Done").length, 61);
});

// T5: the highest Controller receipt seq wins; a producer revision alone does
// not reorder stamped snapshots.
test("T5 the highest observedSeq is shown", () => {
  const rows = derive([
    subagent(1, "a", "running", 50, { observedSeq: 1 }),
    subagent(2, "a", "idle", 60, { observedSeq: 9 }),
    // A delayed live frame with a higher revision but an older receipt seq.
    subagent(3, "a", "running", 70, { observedSeq: 5 }),
  ]);
  assertEquals(chips(rows), { a: "Done" });
});

// T6: snapshots of the previous worker incarnation are fenced: not reporting
// and no Stop until the new incarnation announces the child again.
test("T6 a worker change fences older-incarnation snapshots", async () => {
  const timeline = [
    subagent(1, "a", "running", 1, { observedSeq: 1, incarnation: "worker-1" }),
    { session_id: "s", seq: 2, kind: "lifecycle", status: "starting", detail: null } as Envelope,
  ];
  const fenced = derive(timeline).find((item) => item.kind === "subagent");
  if (fenced?.kind !== "subagent") throw new Error("expected a row");
  assertEquals(subagentStatus(fenced.subagent).group, "not_reporting");
  assertEquals(fenced.subagent.cancellable, false);
  // The durable row sits at its anchor before the boundary but was observed
  // after it: it is not fenced by position (the pre-fix bug after a reload).
  const restored = derive([
    subagent(1, "a", "running", 2, { observedSeq: 3, incarnation: "worker-2" }),
    { session_id: "s", seq: 2, kind: "lifecycle", status: "starting", detail: null } as Envelope,
  ]).find((item) => item.kind === "subagent");
  if (restored?.kind !== "subagent") throw new Error("expected a row");
  assertEquals(subagentStatus(restored.subagent).group, "active");
  assertEquals(restored.subagent.cancellable, true);
  // The bootstrap names the current incarnation: an older one is fenced even
  // without a boundary row in the loaded window.
  const window = [subagent(10, "a", "running", 1, { observedSeq: 10, incarnation: "worker-1" })];
  await reconcileSubagents("t6", state([], { incarnation: "worker-2" }), window);
  const row = applySubagentAuthority(derive(window), sessionAuthority("t6"))
    .find((item) => item.kind === "subagent");
  if (row?.kind !== "subagent") throw new Error("expected a row");
  assertEquals(subagentStatus(row.subagent).label, "Not reporting");
  assertEquals(subagentSummary(row.subagent).canStop, false);
});

// T7: a resumed child keeps one row whose chip follows the latest state.
test("T7 a resumed child is one row following its latest state", () => {
  const timeline = [
    subagent(1, "a", "idle", 1, { observedSeq: 1 }),
    subagent(2, "a", "running", 2, { observedSeq: 2 }),
    subagent(3, "a", "idle", 3, { observedSeq: 3 }),
  ];
  const rows = derive(timeline).filter((item) => item.kind === "subagent");
  assertEquals(rows.length, 1);
  assertEquals(chips(rows), { a: "Done" });
});

// T13: not reporting is not ended.
test("T13 Not reporting is non-terminal; Ended is terminal", () => {
  const quiet = subagentSummary(subagentOf(subagentUpdate("q", "running", 1, { reporting: false }) as AcpUpdate)!);
  const ended = subagentSummary(subagentOf(subagentUpdate("e", "idle", 1, { stopReason: null }) as AcpUpdate)!);
  assertEquals([quiet.chip, quiet.group], ["Not reporting", "not_reporting"]);
  assertEquals([ended.chip, ended.group], ["Ended: outcome unreported", "ended"]);
  const sections = groupTasks([quiet, ended]).map((entry) => [entry.section, entry.tasks.map((task) => task.key)]);
  assertEquals(sections, [["active", ["subagent:q"]], ["ended", ["subagent:e"]]]);
});

function call(overrides: Partial<ManagedCallSummary> = {}): ManagedCallSummary {
  return {
    call_id: "call-1",
    request_id: "op-1",
    provider: "codex",
    purpose: "review",
    labels: { title: "auth fallback" },
    placement: { parent_session_id: "s", machine_id: "ovh", workspace_id: "w" },
    child_session_id: "child-1",
    state: "running",
    created_at_ms: 1_000,
    updated_at_ms: 2_000,
    started_at_ms: 1_000,
    finished_at_ms: null,
    input_revision: null,
    has_result: false,
    runtime_machine_id: null,
    provider_version: null,
    cancel_requested: false,
    error: null,
    ...overrides,
  };
}

// T12: the ledger is authoritative; a disconnected Machine is a health badge,
// never a terminal claim.
test("T12 a call keeps Working with a Reconnecting badge while its Machine is away", () => {
  const away = callSummary(call(), { machineUnreachable: true, now: 101_000 });
  assertEquals([away.chip, away.health, away.group], ["Working", "Reconnecting", "active"]);
  const failed = callSummary(call({ state: "failed", error: { code: "provider_unavailable" }, finished_at_ms: 3_000 }));
  assertEquals([failed.chip, failed.group, failed.attention], ["Failed: provider unavailable", "failed", "failed_unseen"]);
  assertEquals(
    callSummary(call({ state: "failed", finished_at_ms: 3_000 }), { acknowledged: () => true }).attention,
    "none",
  );
  assertEquals(callSummary(call({ state: "completed", finished_at_ms: 3_000 })).chip, "Done");
});

// T11 (web): only a structured MCP identity links an anchor to its tool card.
test("T11 an MCP-origin call links to its tool card; a shell call does not", () => {
  const items: RenderItem[] = derive([
    {
      session_id: "s",
      seq: 1,
      kind: "update",
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "t1",
        title: "cowboy_execute",
        status: "completed",
        rawInput: { action: "review.start", operation: "op-1", arguments: {} },
      } as AcpUpdate,
    },
    {
      session_id: "s",
      seq: 2,
      kind: "update",
      update: {
        sessionUpdate: "cowboy_call_anchor",
        callId: "call-1",
        requestId: "op-1",
        origin: { kind: "mcp", action: "review.start", operation: "op-1" },
      } as AcpUpdate,
    },
    {
      session_id: "s",
      seq: 3,
      kind: "update",
      update: {
        sessionUpdate: "cowboy_call_anchor",
        callId: "call-1",
        requestId: "op-1",
        origin: { kind: "mcp", action: "review.start", operation: "op-1" },
      } as AcpUpdate,
    },
  ]);
  const anchors = items.filter((item) => item.kind === "call");
  assertEquals(anchors.length, 1, "a repeated anchor is one row");
  assertEquals(originToolKey(items, "op-1"), "1");
  assertEquals(originToolKey(items, "cli-7"), null);
});

// §6.3: the bar counts TaskSummary items only; the legacy aggregate is a
// neutral mark, never added.
test("the bar never adds the legacy background count", () => {
  const working = subagentSummary(subagentOf(subagentUpdate("a", "running", 1) as AcpUpdate)!);
  assertEquals(barSummary([working], 1).extraActivity, false, "max(native, subagents) = subagents");
  const bar = barSummary([working], 3);
  assertEquals([bar.working, bar.extraActivity, bar.visible], [1, true, true]);
  assertEquals(barSummary([], 0).visible, false);
  assertEquals(barSummary([callSummary(call({ state: "completed" }))], 0).visible, false, "history stays in the list");
});

test("nested attention rolls up so (+N) cannot hide a blocked child", () => {
  const parent = subagentOf(subagentUpdate("p", "running", 1) as AcpUpdate)!;
  const child = subagentOf(subagentUpdate("c", "requires_action", 1, { parentSessionId: "p" }) as AcpUpdate)!;
  const tasks = buildTaskSummaries({
    subagents: [parent, child].map((subagent) => ({ subagent, anchorSeq: null, reconciledBy: null, reconciledAt: null })),
    calls: [],
    approval: null,
    pending: [{
      seq: 5,
      requestId: "r",
      requestKind: "permission",
      title: "Run",
      options: [],
      elicitation: null,
      subagent: "Agent c",
      subagentId: "c",
    }],
  });
  const byKey = new Map(tasks.map((task) => [task.key, task]));
  assertEquals(byKey.get("subagent:c")?.chip, "Needs your answer");
  assertEquals(byKey.get("subagent:p")?.childAttention, 1);
  assertEquals(byKey.get("subagent:p")?.childCount, 1);
});

// T17: a Controller without the contract leaves cached rows as rendered.
test("T17 an older Controller falls back to the cached rows", async () => {
  const timeline = [subagent(1, "a", "running", 1)];
  const report = await reconcileSubagents("t17", undefined, timeline);
  assertEquals(report.reconciledBy, "fallback");
  assertEquals(sessionAuthority("t17")?.supported, false);
  assertEquals(chips(applySubagentAuthority(derive(timeline), sessionAuthority("t17"))), { a: "Working" });
});

// T10 (web): an answer binds to the presented requests.
test("T10 an approval answer sends exactly the presented set", () => {
  const approval: CallApproval = {
    schema: 1,
    caller: "claude-code",
    reason: "calls_disabled",
    requests: 1,
    agents: ["codex"],
    items: [{ approval_request_id: "apr-1", agent: "codex", purpose: "review", summary: "Review" }],
    request_ids: ["apr-1"],
    prompt_revision: 4,
    ttl_ms: 15_000,
  };
  assertEquals(callApprovalDecisionBody("once", approval), { decision: "once", presented: ["apr-1"], prompt_revision: 4 });
  const legacy = { ...approval } as Partial<CallApproval>;
  delete legacy.request_ids;
  assertEquals(callApprovalDecisionBody("once", legacy as CallApproval), { decision: "once" });
});

test("task deep links round-trip and reject other hashes", () => {
  assertEquals(taskKeyFromHash(taskHash("subagent:child-1")), "subagent:child-1");
  assertEquals(taskKeyFromHash("#task=call%3Acall-1&x=1"), "call:call-1");
  assertEquals(taskKeyFromHash("#task=evil"), null);
  assertEquals(taskKeyFromHash("#other"), null);
});
