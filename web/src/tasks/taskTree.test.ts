import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { buildTaskSummaries, callKey, subagentKey } from "../backgroundTasks.ts";
import { type ManagedCallDetail, type ManagedCallSummary, projectCallSummaryFromDetail } from "../managedCalls.ts";
import type { Subagent } from "../subagents.ts";
import {
  buildTaskForest,
  callVerdictLabel,
  compareTaskOrder,
  flattenTaskSections,
  formatTaskQuote,
  isTaskEnded,
  taskHeadline,
  taskTabSummary,
  taskTimeLabel,
} from "./taskTree.ts";
import { stopGrantKey } from "./useBoundStopConfirm.ts";

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

test("compareTaskOrder is strictly transitive across mixed anchored, unanchored, and approval tasks", () => {
  const tasks = buildTaskSummaries({
    subagents: [
      { subagent: subagent({ id: "s-anch-30", observedSeq: 30 }), anchorSeq: 30, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "s-anch-10", observedSeq: 10 }), anchorSeq: 10, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "s-unanch-90", observedSeq: 90 }), anchorSeq: null, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "s-unanch-40", observedSeq: 40 }), anchorSeq: null, reconciledBy: "live", reconciledAt: 100 },
    ],
    calls: [
      call({ call_id: "c-anch-20", created_at_ms: 1_700_000_200_000 }),
      call({ call_id: "c-unanch-old", created_at_ms: 1_700_000_010_000 }),
      call({ call_id: "c-unanch-new", created_at_ms: 1_700_000_500_000 }),
    ],
    callAnchors: new Map([["c-anch-20", 20]]),
    approval: {
      schema: 1,
      caller: "claude-code",
      reason: "calls_disabled",
      requests: 1,
      agents: ["codex"],
      prompt_revision: 7,
      request_ids: ["req-app-1"],
      ttl_ms: 60_000,
      items: [{ agent: "codex", purpose: "review", summary: "Review patch" }],
    },
    pending: [],
  });
  // Verify strict weak ordering axioms on all triples (irreflexivity, antisymmetry, transitivity).
  for (const a of tasks) {
    assertEquals(compareTaskOrder(a, a), 0);
    for (const b of tasks) {
      const ab = Math.sign(compareTaskOrder(a, b));
      const ba = Math.sign(compareTaskOrder(b, a));
      assertEquals(ab, ba === 0 ? 0 : -ba);
      for (const c of tasks) {
        const bc = Math.sign(compareTaskOrder(b, c));
        const ac = Math.sign(compareTaskOrder(a, c));
        if (ab < 0 && bc < 0) assertEquals(ac, -1);
        if (ab > 0 && bc > 0) assertEquals(ac, 1);
      }
    }
  }
  const sorted = [...tasks].sort(compareTaskOrder);
  assertEquals(sorted.map((t) => t.key), [
    "approval:req-app-1",
    subagentKey("s-anch-30"),
    callKey("c-anch-20"),
    subagentKey("s-anch-10"),
    callKey("c-unanch-new"),
    callKey("c-unanch-old"),
    subagentKey("s-unanch-90"),
    subagentKey("s-unanch-40"),
  ]);
});

test("isTaskEnded uses execution status rather than Needs you attention grouping", () => {
  const tasks = buildTaskSummaries({
    subagents: [
      { subagent: subagent({ id: "s-fail-unseen", state: "idle", stopReason: "error", error: "crashed" }), anchorSeq: 1, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: "s-live", state: "running" }), anchorSeq: 2, reconciledBy: "live", reconciledAt: 100 },
    ],
    calls: [
      call({ call_id: "c-fail-unseen", state: "failed", finished_at_ms: 1_700_000_090_000, error: { code: "provider_error", detail: "oops" } }),
      call({ call_id: "c-live", state: "running" }),
    ],
    approval: {
      schema: 1,
      caller: "claude-code",
      reason: "calls_disabled",
      requests: 1,
      agents: ["codex"],
      prompt_revision: 1,
      request_ids: ["app-1"],
      ttl_ms: 60_000,
      items: [{ agent: "codex", purpose: "review", summary: "Review" }],
    },
    pending: [],
  });
  const byKey = new Map(tasks.map((t) => [t.key, t]));
  assertEquals(byKey.get(callKey("c-fail-unseen"))!.attention, "failed_unseen");
  assertEquals(isTaskEnded(byKey.get(callKey("c-fail-unseen"))!), true);
  assertEquals(isTaskEnded(byKey.get(subagentKey("s-fail-unseen"))!), true);
  assertEquals(isTaskEnded(byKey.get(callKey("c-live"))!), false);
  assertEquals(isTaskEnded(byKey.get(subagentKey("s-live"))!), false);
  assertEquals(isTaskEnded(byKey.get("approval:app-1")!), false);
});

test("compareTaskOrder distinguishes Unicode keys that collate equally", () => {
  const nfc = "caf\u00e9";
  const nfd = "cafe\u0301";
  assertEquals(nfc === nfd, false);
  assertEquals(nfc.localeCompare(nfd), 0);
  const tasks = buildTaskSummaries({
    subagents: [
      { subagent: subagent({ id: nfc, observedSeq: 10 }), anchorSeq: 10, reconciledBy: "live", reconciledAt: 100 },
      { subagent: subagent({ id: nfd, observedSeq: 10 }), anchorSeq: 10, reconciledBy: "live", reconciledAt: 100 },
    ],
    calls: [],
    approval: null,
    pending: [],
  });
  const [a, b] = tasks;
  const cmp = compareTaskOrder(a!, b!);
  assertEquals(cmp !== 0, true);
  assertEquals(compareTaskOrder(b!, a!), -cmp);
  const forward = [a!, b!].sort(compareTaskOrder).map((t) => t.key);
  const reversed = [b!, a!].sort(compareTaskOrder).map((t) => t.key);
  assertEquals(forward, reversed);
});

test("projectCallSummaryFromDetail preserves full ManagedCallSummary metadata and normalizes has_result", () => {
  const detail: ManagedCallDetail = {
    ...call({
      call_id: "c-reuse",
      state: "completed",
      has_result: false,
      verdict: "needs_attention",
      finding_count: 3,
      reused_from: "c-prev",
      reused_child_session_id: "child-prev",
      cancel_requested: false,
      usage: {
        uncached_input_tokens: 10,
        cached_input_tokens: 5,
        output_tokens: 20,
        total_tokens: 35,
        reasoning_tokens: 4,
      },
    }),
    result: { text: "Found 3 issues", structured: null, truncated: false, artifacts: [] },
    events: [],
    prompt_excerpt: "Review",
  };
  const summary = projectCallSummaryFromDetail(detail);
  assertEquals(summary.has_result, true);
  assertEquals(summary.reused_from, "c-prev");
  assertEquals(summary.reused_child_session_id, "child-prev");
  assertEquals(summary.verdict, "needs_attention");
  assertEquals(summary.finding_count, 3);
  assertEquals(summary.usage?.total_tokens, 35);
  assertEquals("events" in summary, false);
});

test("stopGrantKey invalidates when connection generation advances across reconnects", () => {
  const [task] = buildTaskSummaries({
    subagents: [{ subagent: subagent({ id: "s1", revision: 4, observedSeq: 12 }), anchorSeq: 12, reconciledBy: "live", reconciledAt: 100 }],
    calls: [],
    approval: null,
    pending: [],
  });
  const g1 = stopGrantKey("sess-1", task!, true, 0, 1);
  const g2 = stopGrantKey("sess-1", task!, true, 0, 2);
  assertEquals(g1 !== null && g2 !== null && g1 !== g2, true);
});
