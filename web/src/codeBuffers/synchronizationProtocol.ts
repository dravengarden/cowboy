/** Closed Service observations, not native references or confirmation grants. */
import type { ContentIdentity } from "./content.ts";
import {
  decodeResourceId,
  integer,
  list,
  record,
  requireValue,
  type ResourceId,
} from "./protocol.ts";

declare const synchronizationId: unique symbol;
export type SynchronizationId = string & {
  readonly [synchronizationId]: true;
};
export type SynchronizationState =
  | {
    readonly kind: "prepared" | "pending" | "unknown" | "retired" | "expired";
  }
  | {
    readonly kind: "applied";
    readonly content: ContentIdentity;
    readonly version: readonly {
      readonly replicaId: number;
      readonly timestamp: number;
    }[];
  }
  | {
    readonly kind: "refused";
    readonly reason: "changed" | "source" | "shared";
  };
export interface SynchronizationSnapshot {
  readonly apiVersion: 1;
  readonly operationId: SynchronizationId;
  readonly resourceId: ResourceId;
  readonly purpose: "refresh_from_disk";
  readonly content: ContentIdentity;
  readonly state: SynchronizationState;
  /** Service job in progress, independently of the native state's kind. */
  readonly pending: boolean;
}

function contentIdentity(value: unknown, expected: ContentIdentity) {
  const row = record(value, ["sha256", "utf8Bytes"]);
  requireValue(
    typeof row.sha256 === "string" && /^[0-9a-f]{64}$/.test(row.sha256),
  );
  const utf8Bytes = integer(row.utf8Bytes, 0, 4 * 1024 * 1024);
  requireValue(
    row.sha256 === expected.sha256 && utf8Bytes === expected.utf8Bytes,
  );
  return Object.freeze({ sha256: row.sha256, utf8Bytes });
}

export function decodeSynchronization(
  value: unknown,
  status: number,
  resourceId: ResourceId,
  expectedContent: ContentIdentity,
  operationId?: SynchronizationId,
): SynchronizationSnapshot {
  const row = record(value, [
    "apiVersion",
    "operationId",
    "resourceId",
    "purpose",
    "content",
    "state",
    "pending",
  ]);
  requireValue(
    row.apiVersion === 1 && decodeResourceId(row.resourceId) === resourceId &&
      row.purpose === "refresh_from_disk" &&
      typeof row.operationId === "string" &&
      /^sync-[0-9a-f]{32}-[0-9a-f]{16}$/.test(row.operationId) &&
      !row.operationId.endsWith("-0000000000000000") &&
      (!operationId || operationId === row.operationId),
  );
  const content = contentIdentity(row.content, expectedContent);
  requireValue(row.state && typeof row.state === "object");
  const kind = (row.state as { kind?: unknown }).kind;
  let state: SynchronizationState;
  switch (kind) {
    case "applied": {
      const applied = record(row.state, ["kind", "content", "version"]);
      const matched = contentIdentity(applied.content, content);
      let previous = -1;
      const version = Object.freeze(
        list(applied.version, 256).map((value) => {
          const entry = record(value, ["replicaId", "timestamp"]);
          const replicaId = integer(entry.replicaId, 0, 0xffff);
          const timestamp = integer(entry.timestamp, 1);
          requireValue(replicaId > previous);
          previous = replicaId;
          return Object.freeze({ replicaId, timestamp });
        }),
      );
      state = Object.freeze({ kind, content: matched, version });
      break;
    }
    case "refused": {
      const refused = record(row.state, ["kind", "reason"]);
      requireValue(
        refused.reason === "changed" || refused.reason === "source" ||
          refused.reason === "shared",
      );
      state = Object.freeze({ kind, reason: refused.reason });
      break;
    }
    case "prepared":
    case "pending":
    case "unknown":
    case "retired":
    case "expired":
      record(row.state, ["kind"]);
      state = Object.freeze({ kind });
      break;
    default:
      requireValue(false);
  }
  requireValue(
    typeof row.pending === "boolean" &&
      (row.pending ? status === 202 : status === 200) &&
      !((kind === "retired" || kind === "expired") && row.pending),
  );
  return Object.freeze({
    apiVersion: 1,
    operationId: row.operationId as SynchronizationId,
    resourceId,
    purpose: "refresh_from_disk",
    content,
    state,
    pending: row.pending,
  });
}
