import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  decodeLifecycleHistory,
  lifecycleEntryKey,
  loadLifecycleHistory,
} from "./pluginLifecycle.ts";
import { lifecycleFixture } from "./pluginLifecycle.fixture.ts";

Deno.test("lifecycle projection preserves domain identity, private-free recovery and immutable evidence", () => {
  const value = lifecycleFixture();
  const history = decodeLifecycleHistory(value, "hawk", "victoria");
  assertEquals(history.entries.map(lifecycleEntryKey), [
    "install:operation-shared-fixture",
    "uninstall:operation-shared-fixture",
  ]);
  assert(
    Object.isFrozen(history) && Object.isFrozen(history.admission) &&
      Object.isFrozen(history.entries),
  );
  for (const entry of history.entries) {
    assert(Object.isFrozen(entry) && Object.isFrozen(entry.operation));
  }
  const resolution = history.entries[1];
  assert(
    resolution.kind === "uninstall" && Object.isFrozen(resolution.resolution),
  );
  value.entries[0].operation.plugin_version = "999.0.0";
  assertEquals(history.entries[0].operation.plugin_version, "1.1.0");
});

Deno.test("lifecycle evidence fails closed on authority, new fields, wrong target and duplicate domain IDs", () => {
  const fixture = lifecycleFixture();
  const decode = (value: unknown) =>
    decodeLifecycleHistory(value, "hawk", "victoria");
  for (
    const patch of [
      { execution_authorized: true },
      { schema: "future" },
      { observation: "atomic" },
      { window: "complete" },
      { limit_per_kind: 33 },
      { machine_id: "other" },
      { plugin_id: "other" },
      { admission: { install: true } },
      { permit: "not-authority" },
      { entries: [fixture.entries[0], fixture.entries[0]] },
      { entries: [{ ...fixture.entries[0], kind: "recovery" }] },
      { entries: [{ ...fixture.entries[0], resolution: null }] },
    ]
  ) {
    assertThrows(() => decode({ ...fixture, ...patch }));
  }
  const rows = Array.from(
    { length: 33 },
    (_, index) => ({
      ...fixture.entries[0],
      operation: {
        ...fixture.entries[0].operation,
        operation_id: `operation-fixture-${index}`,
      },
    }),
  );
  assertThrows(() => decode({ ...fixture, entries: rows }));
  assertThrows(() => decodeLifecycleHistory(fixture, "../hawk", "victoria"));
});

Deno.test("independent resolution must match proven pre-effect interruption and never claim restoration", () => {
  const value = lifecycleFixture();
  const row = value.entries[1];
  const decode = (entry: unknown) =>
    decodeLifecycleHistory({ ...value, entries: [entry] }, "hawk", "victoria");
  for (
    const patch of [
      { phase: "needs_attention" },
      { updated_at_ms: 4 },
      { attention_from: "uninstalling" },
      { cause: "machine_rejected" },
      { problem: "preconditions_changed" },
      { affected_session_count: 1025 },
      { evidence_schema: 3 },
      { updated_at_ms: 0 },
      { actor: { user_id: "private" } },
    ]
  ) {
    assertThrows(() =>
      decode({ ...row, operation: { ...row.operation, ...patch } })
    );
  }
  for (
    const patch of [
      { action: "restore" },
      { resolved_at_ms: 4 },
      { plugin_mutation_performed: true },
      { worker_restoration_performed: true },
      { session_mutation_performed: true },
      { operation_digest: "private" },
    ]
  ) {
    assertThrows(() =>
      decode({ ...row, resolution: { ...row.resolution, ...patch } })
    );
  }
  decode({ ...row, resolution: null }); // absence is not fabricated recovery
});

Deno.test("unified history reuses the strict Machine receipt/Service-phase decoder", () => {
  const value = lifecycleFixture();
  const row = value.entries[0];
  for (
    const receipt of [null, { state: "pending", phase: "staging" }, {
      state: "applied",
      revision: "not-an-installation",
    }, {
      state: "applied",
      revision: `installation-${"a".repeat(64)}`,
      token: "private",
    }]
  ) {
    assertThrows(() =>
      decodeLifecycleHistory(
        {
          ...value,
          entries: [{
            ...row,
            operation: { ...row.operation, machine_receipt: receipt },
          }],
        },
        "hawk",
        "victoria",
      )
    );
  }
});

Deno.test("loading lifecycle evidence is bounded no-store GET only and preserves caller cancellation", async () => {
  const previous = globalThis.fetch;
  const calls: RequestInit[] = [];
  let response = Response.json(lifecycleFixture());
  globalThis.fetch = ((url, init) => {
    assertEquals(url, "/api/machines/hawk/plugins/victoria/lifecycle-history");
    calls.push(init ?? {});
    return Promise.resolve(response);
  }) as typeof fetch;
  try {
    const owner = new AbortController();
    assertEquals(
      (await loadLifecycleHistory("hawk", "victoria", owner.signal)).entries
        .length,
      2,
    );
    assertEquals(calls[0].method ?? "GET", "GET");
    assertEquals(calls[0].cache, "no-store");
    assertEquals(calls[0].credentials, "same-origin");
    assertEquals(calls[0].body, undefined);
    owner.abort();
    assert(calls[0].signal?.aborted);
    for (
      const body of ["{", " ".repeat(128 * 1024 + 1), new Uint8Array([0xff])]
    ) {
      response = new Response(body, {
        headers: { "content-type": "application/json" },
      });
      await assertRejects(() => loadLifecycleHistory("hawk", "victoria"));
      assert(response.body?.locked === false);
    }
    response = new Response("private error", { status: 503 });
    await assertRejects(() => loadLifecycleHistory("hawk", "victoria"));
  } finally {
    globalThis.fetch = previous;
  }
});
