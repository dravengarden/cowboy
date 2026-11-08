import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { DELEGATION_LIMIT, delegationTask, parseDelegations, withDelegation } from "./reviewDelegations.ts";
import { findingBadge, reviewOutcome } from "./reviewFindingsModel.ts";

const files = [{ path: "src/a.ts", fingerprint: "f1" }, { path: "src/b.ts" }];
const result = (findings: unknown[], extra: Record<string, unknown> = {}) => ({
  structured: { review_complete: true, verdict: "needs-attention", summary: "", findings, ...extra },
});
const finding = (file: string, severity = "high", line = 3) =>
  ({ severity, title: "t", body: "b", file, line_start: line, line_end: line + 1 });

test("a finding is current only while its file is unchanged since the review was asked", () => {
  const outcome = reviewOutcome("c1", result([finding("src/a.ts"), finding("./src/a.ts", "low"), finding("src/b.ts"), finding("src/z.ts")]), files,
    (path) => path === "src/a.ts" ? "f1" : "x");
  assertEquals(outcome.findings.map((f) => [f.path, f.anchor]), [
    ["src/a.ts", "current"], ["src/a.ts", "current"], ["src/b.ts", "stale"], ["src/z.ts", "unmapped"],
  ]);
  const changed = reviewOutcome("c1", result([finding("src/a.ts")]), files, () => "f2");
  assertEquals(changed.findings[0]!.anchor, "stale");
});

test("an incomplete, truncated or malformed review anchors nothing", () => {
  assertEquals(reviewOutcome("c", result([finding("src/a.ts")], { review_complete: false }), files, () => "f1").complete, false);
  assertEquals(reviewOutcome("c", { ...result([finding("src/a.ts")]), truncated: true }, files, () => "f1").findings, []);
  assertEquals(reviewOutcome("c", result([finding("src/a.ts")], { verdict: "incomplete" }), files, () => "f1").complete, false);
  assertEquals(reviewOutcome("c", null, files, () => "f1").complete, false);
  assertEquals(reviewOutcome("c", result([{ file: "src/a.ts", severity: "weird", line_start: 1 }]), files, () => "f1").findings, []);
});

test("the Navigator badge counts current findings and names the worst", () => {
  const { findings } = reviewOutcome("c", result([finding("src/a.ts", "medium"), finding("src/a.ts", "high"), finding("src/b.ts", "critical")]), files,
    (path) => path === "src/a.ts" ? "f1" : undefined);
  assertEquals(findingBadge(findings, "src/a.ts"), { count: 2, severity: "high" });
  assertEquals(findingBadge(findings, "src/b.ts"), undefined);
});

test("delegations are validated, deduplicated by task and bounded", () => {
  let store = {};
  for (let i = 0; i <= DELEGATION_LIMIT; i++) {
    store = withDelegation(store, "s", { fork: "f", task: `t${String(i)}`, kind: "review", files: [], atMs: i });
  }
  assertEquals((store as Record<string, unknown[]>).s!.length, DELEGATION_LIMIT);
  assertEquals(parseDelegations({ s: [{ fork: 1 }, { fork: "f", task: "t", kind: "review", files: [{ path: "a" }], atMs: 1 }] }).s!.length, 1);
  assertEquals(/^[A-Za-z0-9._-]{1,128}$/.test(delegationTask(1794148174474, 0.5)), true);
});
