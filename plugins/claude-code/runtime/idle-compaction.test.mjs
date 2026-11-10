import { test } from "node:test";
import assert from "node:assert/strict";
import { disableIdleCompaction, IDLE_COMPACTION_POLICY } from "./idle-compaction.mjs";

test("idle compaction never arms, and an upstream reason still wins", () => {
  class IdleCompaction {
    reason;
    disabledBy() {
      return this.reason;
    }
  }
  disableIdleCompaction(IdleCompaction);
  const gate = new IdleCompaction();
  assert.equal(gate.disabledBy(), IDLE_COMPACTION_POLICY);
  gate.reason = "idleCompaction setting";
  assert.equal(gate.disabledBy(), "idleCompaction setting");
});

test("a changed upstream gate fails closed instead of compacting", () => {
  assert.throws(() => disableIdleCompaction(class {}), /idle compaction gate/);
  assert.throws(() => disableIdleCompaction(undefined), /idle compaction gate/);
});

test("the pinned adapter's idle compaction is disabled before any session", async () => {
  const upstream = new URL(
    "../../../dist/claude-source-tests/node_modules/@agentclientprotocol/claude-agent-acp/dist/idle-compaction.js",
    import.meta.url,
  );
  const { IdleCompaction } = await import(upstream);
  disableIdleCompaction(IdleCompaction);
  const timers = [];
  const gate = new IdleCompaction(
    {
      isBusy: () => false,
      contextTokens: () => 900_000,
      settings: () => ({}),
      compact: () => assert.fail("idle compaction must not send /compact"),
      log: () => {},
    },
    { env: {}, setTimeout: (callback) => timers.push(callback) },
  );
  gate.noteRequest({ cache_creation: { ephemeral_1h_input_tokens: 1 } }, "message");
  gate.onIdle();
  assert.deepEqual(timers, []);
  assert.equal(gate.skipReason(), `disabled by ${IDLE_COMPACTION_POLICY}`);
  await gate.fire();
  assert.equal(gate.isCompacting, false);
});
