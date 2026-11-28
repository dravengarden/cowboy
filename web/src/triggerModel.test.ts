import { test } from "bun:test";
import assert from "node:assert/strict";
import type { SessionMeta } from "./protocol";
import {
  formatTriggerAge,
  isAttentionTrigger,
  isSettledTrigger,
  isTriggerSession,
  mergeReorderedTriggers,
  summarizeTriggers,
  triggerContextPercent,
  triggerDisplayMeta,
  triggerOutcomeMeta,
  triggerTimestampMs,
} from "./triggerModel";

function mk(overrides: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id: "sess-1",
    provider: "codex",
    cwd: "/work",
    title: "Normal session",
    status: "exited",
    origin: "web",
    ...overrides,
  };
}

test("classifies structured and legacy trigger sessions separately from ordinary web sessions", () => {
  const webSession = mk({ id: "web-1" });
  const structured = mk({
    id: "trig-1",
    origin: "api",
    title: "[monitor] Disk usage high",
    trigger: { kind: "alert.v1", source: "monitor", outcome: "resolved" },
  });
  const legacy = mk({
    id: "trig-2",
    origin: "api",
    title: "[External Intake: alert.v1]",
  });
  const trashed = mk({
    id: "trig-3",
    origin: "api",
    trashed_at_ms: 1000,
    trigger: { kind: "alert.v1", source: "monitor", outcome: "resolved" },
  });
  assert.equal(isTriggerSession(webSession), false);
  assert.equal(isTriggerSession(structured), true);
  assert.equal(isTriggerSession(legacy), true);
  assert.equal(isTriggerSession(trashed), false);
});

test("preserves trigger display order for drag-reorder and tracks activity + settled ids", () => {
  const settled = mk({
    id: "s-resolved",
    origin: "api",
    status: "exited",
    trigger: { kind: "k", source: "src", outcome: "resolved" },
  });
  const escalated = mk({
    id: "s-esc",
    origin: "api",
    status: "exited",
    trigger: { kind: "k", source: "src", outcome: "escalated" },
  });
  const running = mk({
    id: "s-run",
    origin: "api",
    status: "busy",
    trigger: { kind: "k", source: "src", outcome: "running" },
  });
  const normal = mk({ id: "w-1", origin: "web" });
  const summary = summarizeTriggers([normal, settled, escalated, running]);
  assert.deepEqual(
    summary.sessions.map((s) => s.id),
    ["s-resolved", "s-esc", "s-run"],
  );
  assert.deepEqual(summary.activity, { working: 1, attention: 1, live: 0 });
  assert.deepEqual(summary.settledIds, ["s-resolved"]);
  assert.equal(isAttentionTrigger(escalated), true);
  assert.equal(isSettledTrigger(settled), true);
  assert.equal(triggerOutcomeMeta(escalated).label, "Escalated");

  assert.deepEqual(
    mergeReorderedTriggers([normal, settled, escalated, running], ["s-run", "s-resolved", "s-esc"]),
    ["w-1", "s-run", "s-resolved", "s-esc"],
  );
});

test("extracts clean trigger display titles, context percentage, and relative age", () => {
  const structured = mk({
    id: "t-1",
    origin: "api",
    title: "[monitor] Disk usage high",
    trigger: {
      kind: "alert.v1",
      source: "monitor",
      correlation_key: "ep-disk-1",
      outcome: "resolved",
      settled_at_ms: 120_000,
    },
    usage: { used: 36_000, size: 200_000, raw: null, observed_at_ms: 100_000 },
  });
  assert.deepEqual(triggerDisplayMeta(structured), {
    title: "Disk usage high",
    source: "monitor",
    kind: "alert.v1",
    correlationKey: "ep-disk-1",
  });
  assert.equal(triggerContextPercent(structured), 18);
  assert.equal(triggerTimestampMs(structured), 120_000);

  const legacy = mk({
    id: "t-2",
    origin: "api",
    title: "[External Intake: alert.triage.v1]",
    context_used: 50,
    context_size: 200,
  });
  assert.deepEqual(triggerDisplayMeta(legacy), {
    title: "alert.triage.v1",
    source: "alert.triage.v1",
    kind: "alert.triage.v1",
    correlationKey: null,
  });
  assert.equal(triggerContextPercent(legacy), 25);

  assert.equal(formatTriggerAge(15_000), "now");
  assert.equal(formatTriggerAge(12 * 60_000), "12m");
  assert.equal(formatTriggerAge(3 * 3_600_000), "3h");
  assert.equal(formatTriggerAge(49 * 3_600_000), "2d");
});
