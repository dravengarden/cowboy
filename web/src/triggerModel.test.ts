import { test } from "bun:test";
import assert from "node:assert/strict";
import type { SessionMeta } from "./protocol";
import {
  isAttentionTrigger,
  isSettledTrigger,
  isTriggerSession,
  summarizeTriggers,
  triggerOutcomeMeta,
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

test("orders running and escalated triggers first and tracks settled bulk-cleanable ids", () => {
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
  const summary = summarizeTriggers([settled, escalated, running]);
  assert.deepEqual(
    summary.sessions.map((s) => s.id),
    ["s-run", "s-esc", "s-resolved"],
  );
  assert.deepEqual(summary.activity, { working: 1, attention: 1, live: 0 });
  assert.deepEqual(summary.settledIds, ["s-resolved"]);
  assert.equal(isAttentionTrigger(escalated), true);
  assert.equal(isSettledTrigger(settled), true);
  assert.equal(triggerOutcomeMeta(escalated).label, "Escalated");
});
