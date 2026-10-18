import { test } from "bun:test";
import { strict as assert } from "node:assert";
import {
  callsOverview,
  callStateLabel,
  callTitle,
  callModelLabel,
  callTimesTitle,
  isManagedChild,
  type ManagedCallSummary,
  managedChildParent,
  reviewFindings,
  startedLabel,
} from "./managedCalls";
import type { SessionMeta } from "./protocol";

function call(
  state: ManagedCallSummary["state"],
  extra: Partial<ManagedCallSummary> = {},
): ManagedCallSummary {
  return {
    call_id: `call-${state}`,
    request_id: "review-r1",
    provider: "codex",
    purpose: "review",
    labels: { round: "2", aspect: "security", group: "review-1" },
    placement: {
      parent_session_id: "parent",
      machine_id: "hawk",
      workspace_id: "w",
    },
    child_session_id: "child",
    state,
    created_at_ms: 0,
    updated_at_ms: 1,
    input_revision: null,
    has_result: false,
    runtime_machine_id: "ovh",
    provider_version: "3.4.0",
    cancel_requested: false,
    error: null,
    ...extra,
  };
}

test("managed children are recognised only from their exact binding", () => {
  const child = {
    id: "child",
    execution_binding: {
      schema: 1,
      phase: "managed_child",
      parent_session_id: "parent",
    },
  } as unknown as SessionMeta;
  assert.equal(managedChildParent(child), "parent");
  assert.ok(isManagedChild(child));
  const remote = {
    id: "remote",
    execution_binding: { schema: 1, runtime: {}, environment: {} },
  } as unknown as SessionMeta;
  assert.equal(managedChildParent(remote), null);
  assert.equal(managedChildParent(undefined), null);
  // A child whose Agent runtime is on another Machine is still a child.
  const split = {
    id: "split",
    execution_binding: {
      schema: 1,
      runtime: {},
      environment: {},
      managed: { parent_session_id: "parent", profile: "read_only_v1" },
    },
  } as unknown as SessionMeta;
  assert.equal(managedChildParent(split), "parent");
  assert.ok(isManagedChild(split));
});

test("review metadata distinguishes observed configuration, selected preset and missing history", () => {
  assert.equal(callModelLabel(call("completed")), "Model not recorded");
  assert.equal(callModelLabel(call("completed", { preset: "fable-max" })), "Selected: fable-max");
  assert.equal(callModelLabel(call("completed", { configuration: { model: "claude-fable-5-1", effort: "max" } })), "claude-fable-5-1 · max");
  assert.match(callTimesTitle(call("failed")), /Start not recorded.*Finish not recorded/);
  const timed = callTimesTitle(call("completed", { started_at_ms: 1000, finished_at_ms: 2000 }));
  assert.match(timed, /Created .*Started .*Finished /);
  assert.equal(callTitle(call("completed", { labels: {} })), "review");
});

test("execution status, verdicts and findings stay separate facts", () => {
  const completed = call("completed", {
    verdict: "needs-attention",
    finding_count: 2,
  });
  const overview = callsOverview([call("running"), completed, call("failed")]);
  assert.deepEqual(
    {
      total: overview.total,
      active: overview.active,
      failed: overview.failed,
      findings: overview.findings,
    },
    { total: 3, active: 1, failed: 1, findings: 2 },
  );
  assert.equal(callStateLabel(completed), "Completed");
  assert.equal(
    callStateLabel(call("running", { cancel_requested: true })),
    "Stopping",
  );
  assert.equal(callTitle(completed), "security · round 2");
});

test("review findings use the common structured shape only", () => {
  const findings = reviewFindings({
    verdict: "needs-attention",
    findings: [{
      severity: "high",
      title: "Unchecked input",
      body: "b",
      file: "src/a.rs",
      line_start: 7,
    }],
  });
  assert.deepEqual(findings, [{
    title: "Unchecked input",
    body: "b",
    severity: "high",
    location: "src/a.rs:7",
  }]);
  assert.equal(reviewFindings({ answer: 42 }), null);
  assert.equal(reviewFindings("plain text"), null);
});

test("a call's start is as short as it stays unambiguous", () => {
  const now = new Date(2026, 9, 12, 21, 56).getTime();
  assert.equal(startedLabel(new Date(2026, 9, 12, 9, 5).getTime(), now), "09:05");
  assert.equal(
    startedLabel(new Date(2026, 9, 11, 22, 32).getTime(), now),
    "Yesterday 22:32",
  );
  assert.equal(
    startedLabel(new Date(2026, 9, 9, 22, 32).getTime(), now),
    "Oct 9 22:32",
  );
  assert.equal(
    startedLabel(new Date(2025, 11, 30, 8, 0).getTime(), now),
    "Dec 30, 2025 08:00",
  );
});
