import { assert, assertEquals, assertThrows } from "jsr:@std/assert";
import { BufferClientError, decodeResourceId } from "./protocol.ts";
import { decodeSynchronization } from "./synchronizationProtocol.ts";
import { appliedState, golden, syncWire } from "./synchronizationFixture.ts";
import { capturedIdentity } from "./content.ts";
import { content } from "./synchronizationFixture.ts";
import budget from "../../../contracts/code-buffer-sync-budget.fixture.json" with {
  type: "json",
};

const id = decodeResourceId(golden.resourceId);
const decode = (value: unknown, status = 200) =>
  decodeSynchronization(value, status, id, golden.content);

Deno.test("Service synchronization fixture matches a real captured UTF-8 digest and freezes complete evidence", async () => {
  assertEquals(capturedIdentity(await content()), golden.content);
  const result = decode(golden);
  assertEquals(result, golden);
  assert(
    Object.isFrozen(result) && Object.isFrozen(result.content) &&
      Object.isFrozen(result.state),
  );
  assert(result.state.kind === "applied");
  assert(
    Object.isFrozen(result.state.version) &&
      Object.isFrozen(result.state.version[0]),
  );
});

Deno.test("synchronization codec rejects wrong domains, unknown fields, owners, content, versions and inconsistent status", () => {
  const bad: unknown[] = [
    null,
    [],
    { ...golden, extra: true },
    { ...golden, apiVersion: 2 },
    { ...golden, purpose: "write_file" },
    { ...golden, operationId: golden.resourceId },
    { ...golden, operationId: `sync-${"a".repeat(32)}-0000000000000000` },
    { ...golden, resourceId: golden.operationId },
    { ...golden, resourceId: `${"a".repeat(32)}-0000000000000002` },
    { ...golden, pending: true },
    { ...golden, pending: 0 },
    { ...golden, content: { ...golden.content, sha256: "A".repeat(64) } },
    { ...golden, content: { ...golden.content, utf8Bytes: 4 } },
    { ...golden, state: { kind: "retired", reason: "none" } },
    { ...golden, state: { kind: "refused", reason: "dirty" } },
    { ...golden, state: { kind: "future" } },
    {
      ...golden,
      state: { ...golden.state, content: { ...golden.content, utf8Bytes: 4 } },
    },
  ];
  for (
    const version of [
      [{ replicaId: 1, timestamp: 0 }],
      [{ replicaId: 65536, timestamp: 1 }],
      [{ replicaId: 0, timestamp: 4294967296 }],
      [{ replicaId: 0.5, timestamp: 1 }],
      [{ replicaId: 1, timestamp: 1 }, { replicaId: 0, timestamp: 2 }],
      [{ replicaId: 1, timestamp: 1 }, { replicaId: 1, timestamp: 2 }],
      Array.from(
        { length: 257 },
        (_, replicaId) => ({ replicaId, timestamp: 1 }),
      ),
    ]
  ) bad.push({ ...golden, state: { ...golden.state, version } });
  for (const value of bad) {
    assertThrows(() => decode(value), BufferClientError, "protocol");
  }
  assertThrows(() => decode(golden, 202), BufferClientError);
  assertThrows(
    () =>
      decodeSynchronization(golden, 200, id, {
        ...golden.content,
        utf8Bytes: 4 * 1024 * 1024 + 1,
      }),
    BufferClientError,
  );
  const other = decode({
    ...golden,
    operationId: `sync-${"a".repeat(32)}-0000000000000002`,
  });
  assertThrows(
    () =>
      decodeSynchronization(golden, 200, id, golden.content, other.operationId),
    BufferClientError,
  );
});

Deno.test("native pending and Service pending are independent, closed observations", () => {
  for (
    const kind of [
      "prepared",
      "pending",
      "unknown",
      "retired",
      "expired",
    ] as const
  ) {
    assertEquals(decode(syncWire({ kind })).state.kind, kind);
    if (kind === "retired" || kind === "expired") {
      assertThrows(
        () => decode(syncWire({ kind }, true), 202),
        BufferClientError,
      );
    } else assert(decode(syncWire({ kind }, true), 202).pending);
  }
  for (const reason of ["changed", "source", "shared", "budget"] as const) {
    assertEquals(decode(syncWire({ kind: "refused", reason })).state, {
      kind: "refused",
      reason,
    });
  }
  assertEquals(decode(syncWire(appliedState)).state, appliedState);
});

Deno.test("actual Service budget fixture has no partial result, retry permission or foreign identity", () => {
  assertEquals(decode(budget), budget);
  for (const field of ["content", "version", "retryAfter", "partial"]) {
    assertThrows(
      () => decode({ ...budget, state: { ...budget.state, [field]: true } }),
      BufferClientError,
    );
  }
  for (const reason of ["Budget", "capacity", "timeout", "future"]) {
    assertThrows(
      () => decode({ ...budget, state: { ...budget.state, reason } }),
      BufferClientError,
    );
  }
  assertThrows(
    () =>
      decode({ ...budget, resourceId: "a".repeat(32) + "-0000000000000002" }),
    BufferClientError,
  );
});
