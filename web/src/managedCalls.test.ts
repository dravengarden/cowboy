import { test } from "bun:test";
import { strict as assert } from "node:assert";
import {
  callsOverview,
  callStateLabel,
  callTitle,
  isManagedChild,
  type ManagedCallSummary,
  managedChildParent,
  reviewFindings,
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
