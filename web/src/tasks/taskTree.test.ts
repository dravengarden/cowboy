import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { buildTaskSummaries, callKey, subagentKey } from "../backgroundTasks.ts";
import type { ManagedCallSummary } from "../managedCalls.ts";
import type { Subagent } from "../subagents.ts";
import {
  buildTaskForest,
  callVerdictLabel,
  compareTaskOrder,
  flattenTaskSections,
  formatTaskQuote,
  taskHeadline,
  taskTabSummary,
  taskTimeLabel,
} from "./taskTree.ts";

function subagent(overrides: Partial<Subagent> = {}): Subagent {
  return {
    id: "child-1",
    parentId: null,
    title: "Explore parser",
    untitled: false,
    description: "Inspect cache behavior",
    prompt: "Find parser cache flake",
    state: "running",
    stopReason: null,
    cancellable: true,
    reporting: true,
    unavailable: false,
    incarnation: "inc-1",
    revision: 10,
    observedSeq: 10,
    terminalSeq: null,
    toolCalls: 2,
    messages: 1,
    omittedActivity: 0,
    model: null,
    tokens: 0,
    activity: [],
    result: null,
    error: null,
    ...overrides,
  };
}

function call(overrides: Partial<ManagedCallSummary> = {}): ManagedCallSummary {
  return {
    call_id: "call-1",
    request_id: "req-1",
    provider: "codex",
    purpose: "review",
    labels: { title: "Store review" },
    placement: { parent_session_id: "s", machine_id: "hawk", workspace_id: "w" },
    child_session_id: "child-call-1",
    configuration: { model: "gpt-6.2-astra", reasoning_effort: "max", permission_mode: "read-only" },
    state: "running",
    created_at_ms: 1_700_000_000_000,
    updated_at_ms: 1_700_000_060_000,
    started_at_ms: 1_700_000_005_000,
    finished_at_ms: null,
    input_revision: null,
    has_result: false,
    runtime_machine_id: "hawk",
    provider_version: null,
    cancel_requested: false,
    error: null,
    ...overrides,
  };
}

test("taskTabSummary separates entry lifetime from live/attention decoration", () => {
  const empty = taskTabSummary([], 0);
  assertEquals(empty.hasTasks, false);
  assertEquals(empty.quietHistory, false);

  const completedOnly = buildTaskSummaries({
    subagents: [{
      subagent: subagent({ id: "done-1", state: "idle", stopReason: "end_turn", result: "All good" }),
      anchorSeq: 5,
      reconciledBy: "live",
      reconciledAt: 100,
    }],
    calls: [],
    approval: null,
    pending: [],
  });
  const quiet = taskTabSummary(completedOnly, 0);
  assertEquals(quiet.hasTasks, true);
  assertEquals(quiet.quietHistory, true);
  assertEquals(quiet.activeCount, 0);
  assertEquals(quiet.endedCount, 1);

  const mixed = buildTaskSummaries({
    subagents: [
      { subagent: subagent({ id: "w1", state: "requires_action" }), anchorSeq: 1, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "f1", state: "idle", stopReason: "error", error: "boom" }), anchorSeq: 2, reconciledBy: "live", reconciledAt: 100 },
    ],
    calls: [call({ call_id: "c1", state: "running" })],
    approval: null,
    pending: [{ requestId: "p1", subagentId: "w1", title: "Run test", options: [] }],
  });
  const live = taskTabSummary(mixed, 0);
  assertEquals(live.hasTasks, true);
  assertEquals(live.quietHistory, false);
  assertEquals(live.needsYouCount, 1);
  assertEquals(live.activeCount, 1);
  assertEquals(live.failedUnseenCount, 1);
  assertEquals(live.lead?.key, subagentKey("w1"));
});

test("buildTaskForest nests real parentKey trees, guards cycles, and rolls up attention", () => {
  const tasks = buildTaskSummaries({
    subagents: [
      { subagent: subagent({ id: "parent", state: "idle", stopReason: "end_turn" }), anchorSeq: 1, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "child", parentId: "parent", state: "running" }), anchorSeq: 2, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "grandchild", parentId: "child", state: "idle", stopReason: "error", error: "panic" }), anchorSeq: 3, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "orphan", parentId: "missing", state: "idle", stopReason: "end_turn" }), anchorSeq: 4, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "cycA", parentId: "cycB", state: "idle", stopReason: "end_turn" }), anchorSeq: 5, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "cycB", parentId: "cycA", state: "idle", stopReason: "end_turn" }), anchorSeq: 6, reconciledBy: "live", reconciledAt: 100 },
    ],
    calls: [],
    approval: null,
    pending: [],
  });

  const sections = buildTaskForest(tasks);
  assertEquals(sections.map((s) => s.section), ["needs_you", "ended"]);
  const parentNode = sections[0]!.roots[0]!;
  assertEquals(parentNode.task.key, subagentKey("parent"));
  assertEquals(parentNode.descendantCount, 2);
  assertEquals(parentNode.descendantAttention, 1);
  assertEquals(parentNode.children[0]!.task.key, subagentKey("child"));
  assertEquals(parentNode.children[0]!.children[0]!.task.key, subagentKey("grandchild"));

  const collapsed = flattenTaskSections(sections, {
    collapsedKeys: new Set([subagentKey("parent")]),
    hideEnded: true,
  });
  assertEquals(collapsed.map((row) => [row.task.key, row.depth, row.collapsed]), [
    [subagentKey("parent"), 0, true],
  ]);

  // Unseen failure stays visible under Hide ended; pure quiet ended roots (orphan, cycA, cycB) hide!
  const expandedHidden = flattenTaskSections(sections, { hideEnded: true });
  assertEquals(expandedHidden.map((row) => [row.task.key, row.depth]), [
    [subagentKey("parent"), 0],
    [subagentKey("child"), 1],
    [subagentKey("grandchild"), 2],
  ]);

  const all = flattenTaskSections(sections);
  assertEquals(all.length, tasks.length);
});

test("compareTaskOrder keeps shared timeline anchors aligned without mixing clocks", () => {
  const tasks = buildTaskSummaries({
    subagents: [
      { subagent: subagent({ id: "s1", observedSeq: 50 }), anchorSeq: 50, reconciledBy: "live", reconciledAt: 100 },
    ],
    calls: [
      call({ call_id: "c1", created_at_ms: 1_700_000_000_000 }),
    ],
    callAnchors: new Map([["c1", 20]]),
    approval: null,
    pending: [],
  });
  const sorted = [...tasks].sort(compareTaskOrder);
  assertEquals(sorted.map((task) => task.key), [subagentKey("s1"), callKey("c1")]);
});

test("taskHeadline and taskTimeLabel report real activity, verdicts and honest missing evidence", () => {
  const tasks = buildTaskSummaries({
    subagents: [
      {
        subagent: subagent({
          id: "active-tool",
          state: "running",
          activity: [
            { kind: "tool", id: "t1", text: "Read parser.rs", status: "completed" },
            { kind: "tool", id: "t2", text: "cargo test parser", status: "in_progress" },
          ],
        }),
        anchorSeq: 1,
        reconciledBy: "live",
        reconciledAt: 100,
      },
      {
        subagent: subagent({
          id: "last-step",
          state: "running",
          activity: [{ kind: "tool", id: "t1", text: "Read parser.rs", status: "completed" }],
        }),
        anchorSeq: 2,
        reconciledBy: "live",
        reconciledAt: 100,
      },
      {
        subagent: subagent({ id: "done-empty", state: "idle", stopReason: "end_turn", result: null }),
        anchorSeq: 3,
        reconciledBy: "live",
        reconciledAt: 100,
      },
    ],
    calls: [
      call({
        call_id: "rev",
        state: "completed",
        verdict: "needs-attention",
        finding_count: 2,
        has_result: true,
        started_at_ms: 1_700_000_000_000,
        finished_at_ms: 1_700_000_090_000,
      }),
      call({
        call_id: "q",
        state: "queued",
        created_at_ms: 1_700_000_000_000,
        started_at_ms: null,
      }),
    ],
    approval: null,
    pending: [],
  });

  const byKey = new Map(tasks.map((t) => [t.key, t]));
  assertEquals(taskHeadline(byKey.get(subagentKey("active-tool"))!), "cargo test parser");
  assertEquals(taskHeadline(byKey.get(subagentKey("last-step"))!), "Last: Read parser.rs");
  assertEquals(taskHeadline(byKey.get(subagentKey("done-empty"))!), "No result captured");
  assertEquals(taskHeadline(byKey.get(callKey("rev"))!), "Done · Needs attention · 2 findings");
  assertEquals(callVerdictLabel(byKey.get(callKey("rev"))!.call!), "Needs attention · 2 findings");
  assertEquals(taskTimeLabel(byKey.get(subagentKey("active-tool"))!), null);
  assertEquals(taskTimeLabel(byKey.get(callKey("rev"))!, 1_700_000_120_000), "1m 30s");
  assertEquals(taskTimeLabel(byKey.get(callKey("q"))!, 1_700_000_030_000), "queued 30s");

  const quote = formatTaskQuote(byKey.get(callKey("rev"))!, "Retry can repeat the write.");
  assertEquals(
    quote.includes('Regarding background task "Store review" (call rev, Done · Needs attention · 2 findings):') &&
      quote.includes("> Retry can repeat the write."),
    true,
  );
});
