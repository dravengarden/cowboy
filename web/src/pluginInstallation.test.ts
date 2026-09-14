import {
  assert,
  assertEquals,
  assertMatch,
  assertNotEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  createPluginInstallRequest,
  decodeInstallHistory,
  loadInstallHistory,
} from "./pluginInstallation.ts";

function fixture() {
  return {
    schema: "dravengarden.cowboy.plugin-install-history/v1",
    admission_enabled: false,
    execution_authorized: false,
    machine_receipt_available: false,
    requires_reconciliation: true,
    operations: [{
      operation_id: "installation-fixture-0001",
      phase: "needs_attention",
      problem: "interrupted",
      attention_from: "installing",
      plugin_kind: "telemetry_backend",
      plugin_version: "1.1.0",
      generation_digest: `sha256:${"a".repeat(64)}`,
      created_at_ms: 1,
      updated_at_ms: 2,
    }],
  };
}

function typedFixture() {
  const { machine_receipt_available: _legacy, ...base } = fixture();
  return {
    ...base,
    schema: "dravengarden.cowboy.plugin-install-history/v2",
    operations: [{
      ...base.operations[0],
      evidence_schema: 2,
      machine_receipt: null as unknown,
    }],
  };
}

Deno.test("typed history accepts only matching receipt and Service progress; legacy remains explicitly legacy", () => {
  const applied = {
    state: "applied",
    revision: `installation-${"d".repeat(64)}`,
  };
  const unknown = {
    state: "unknown",
    phase: "activating",
    reason: "effect_failure",
  };
  const pending = { state: "pending", phase: "staging" };
  const rejected = { state: "rejected", reason: "target_changed" };
  const input = (changed: Record<string, unknown>) => ({
    ...typedFixture(),
    operations: [{ ...typedFixture().operations[0], ...changed }],
  });
  for (
    const changed of [
      {},
      { evidence_schema: 1 },
      {
        phase: "completed",
        problem: null,
        attention_from: null,
        machine_receipt: applied,
      },
      {
        phase: "aborted",
        problem: "machine_rejected",
        attention_from: null,
        machine_receipt: rejected,
      },
      { machine_receipt: pending, problem: "unknown_machine_outcome" },
      { machine_receipt: unknown, problem: "unknown_machine_outcome" },
      { machine_receipt: applied, attention_from: "machine_acknowledged" },
    ]
  ) {
    const history = decodeInstallHistory(input(changed));
    const receipt = history.operations[0].machine_receipt;
    assert(receipt === null || Object.isFrozen(receipt));
    assertEquals(history.execution_authorized, false);
  }
  for (
    const changed of [
      { evidence_schema: 3 },
      { evidence_schema: 1, machine_receipt: applied },
      { phase: "completed", problem: null, attention_from: null },
      {
        phase: "installing",
        problem: null,
        attention_from: null,
        machine_receipt: applied,
      },
      { phase: "aborted", problem: "machine_rejected", attention_from: null },
      { machine_receipt: applied },
      { machine_receipt: rejected },
      { attention_from: "machine_acknowledged" },
      { machine_receipt: pending },
      {
        machine_receipt: unknown,
        problem: "unknown_machine_outcome",
        attention_from: "prepared",
      },
      {
        machine_receipt: { ...pending, phase: "projecting_authentication" },
        problem: "unknown_machine_outcome",
      },
      {
        machine_receipt: { ...unknown, retry: true },
        problem: "unknown_machine_outcome",
      },
      {
        machine_receipt: { ...applied, revision: "latest" },
        attention_from: "machine_acknowledged",
      },
      { machine_receipt: { state: "restored" } },
    ]
  ) assertThrows(() => decodeInstallHistory(input(changed)));
});

Deno.test("installation evidence is closed, immutable and cannot authorize an effect", () => {
  const history = decodeInstallHistory(fixture());
  assert(Object.isFrozen(history));
  assert(Object.isFrozen(history.operations));
  assert(Object.isFrozen(history.operations[0]));
  const neverAuthorized: false = history.execution_authorized;
  assertEquals(neverAuthorized, false);
  assertEquals(history.operations[0].attention_from, "installing");
  for (
    const input of [
      { ...fixture(), schema: "future" },
      { ...fixture(), execution_authorized: true },
      { ...fixture(), machine_receipt_available: true },
      { ...fixture(), retry: true },
      {
        ...fixture(),
        operations: Array.from({ length: 33 }, () => fixture().operations[0]),
      },
      {
        ...fixture(),
        operations: [fixture().operations[0], fixture().operations[0]],
      },
    ]
  ) assertThrows(() => decodeInstallHistory(input));
  for (
    const changed of [
      { phase: "restored" },
      { phase: "completed" },
      { problem: "private error" },
      { attention_from: "completed" },
      { operation_id: "short" },
      { generation_digest: "latest" },
      { updated_at_ms: 0 },
      { updated_at_ms: Number.MAX_SAFE_INTEGER },
      { created_at_ms: 3 },
      { plugin_kind: "authentication_provider" },
      { actor: "private" },
      { problem: null },
    ]
  ) {
    assertThrows(() =>
      decodeInstallHistory({
        ...fixture(),
        operations: [{ ...fixture().operations[0], ...changed }],
      })
    );
  }
});

Deno.test("historical completion and authentication pending never imply current inventory", () => {
  for (
    const [phase, problem] of [
      ["completed", null],
      ["authentication_pending", "authentication_sync_failed"],
      ["aborted", "transport_not_sent"],
    ]
  ) {
    const history = decodeInstallHistory({
      ...fixture(),
      operations: [{
        ...fixture().operations[0],
        phase,
        problem,
        attention_from: null,
      }],
    });
    assertEquals(history.execution_authorized, false);
    assertEquals(history.operations[0].evidence_schema, 1);
    assertEquals(history.operations[0].machine_receipt, null);
    assertEquals(history.operations[0].phase, phase);
  }
});

Deno.test("one explicit installation action allocates one exact immutable identity", () => {
  const digest = `sha256:${"a".repeat(64)}`;
  const first = createPluginInstallRequest("1.1.0", digest);
  assert(Object.isFrozen(first));
  assertMatch(first.operation_id, /^installation-[0-9a-f-]{36}$/);
  assertNotEquals(
    first.operation_id,
    createPluginInstallRequest("1.1.0", digest).operation_id,
  );
  assertEquals(first.digest, digest);
  assertThrows(() => createPluginInstallRequest("1.1.0", "latest"));
});

Deno.test("reload reads only saved evidence and never posts or retries an installation", async () => {
  const original = globalThis.fetch;
  const calls: RequestInit[] = [];
  const urls: string[] = [];
  globalThis.fetch = (input, init) => {
    calls.push(init ?? {});
    urls.push(String(input));
    return Promise.resolve(Response.json(fixture()));
  };
  try {
    for (let index = 0; index < 2; index++) {
      const history = await loadInstallHistory("machine-test", "victoria");
      assertEquals(history.operations[0].phase, "needs_attention");
    }
    assertEquals(
      urls,
      Array(2).fill(
        "/api/machines/machine-test/plugins/victoria/installation-operations",
      ),
    );
    assert(
      calls.every((init) =>
        !init.method && !init.body && init.cache === "no-store" &&
        init.credentials === "same-origin"
      ),
    );
  } finally {
    globalThis.fetch = original;
  }
});

Deno.test("failed, malformed and oversized history responses are unavailable, never empty or retried", async () => {
  const original = globalThis.fetch;
  try {
    for (
      const response of [
        new Response("private", { status: 503 }),
        new Response("<html>"),
        Response.json({ ...fixture(), execution_authorized: true }),
        new Response(" ".repeat(128 * 1024 + 1), {
          headers: { "content-type": "application/json" },
        }),
      ]
    ) {
      let calls = 0;
      globalThis.fetch = () => {
        calls++;
        return Promise.resolve(response);
      };
      await assertRejects(
        () => loadInstallHistory("machine-test", "victoria"),
        Error,
        "unavailable or incompatible",
      );
      assertEquals(calls, 1);
    }
  } finally {
    globalThis.fetch = original;
  }
});
