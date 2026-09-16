/** Closed core HTTP observations, never native handles or serialized grants. */
declare const resourceId: unique symbol;
export type ResourceId = string & { readonly [resourceId]: true };
export type BufferState = "prepared" | "open" | "released" | "unknown";
export interface Snapshot {
  readonly apiVersion: 1;
  readonly resourceId: ResourceId;
  readonly state: BufferState;
  readonly pending: boolean;
}
export interface Point {
  readonly row: number;
  readonly column: number;
}
export interface Diagnostic {
  readonly start: Point;
  readonly end: Point;
  readonly severity: number;
  readonly source: string | null;
  readonly message: string;
}
export interface InlayHint {
  readonly offset: number;
  readonly label: string;
  readonly kind: string | null;
  readonly paddingLeft: boolean;
  readonly paddingRight: boolean;
}
export interface DocumentSymbol {
  readonly name: string;
  readonly kind: number;
  readonly start: Point;
  readonly end: Point;
  readonly selectionStart: Point;
  readonly selectionEnd: Point;
  readonly children: readonly DocumentSymbol[];
}
export interface Results {
  language: {
    readonly kind: "language";
    readonly diagnosticsState: "unobserved" | "observed";
    readonly diagnostics: readonly Diagnostic[];
    readonly inlayHints: readonly InlayHint[];
    readonly semanticTokens: readonly number[];
  };
  symbols: {
    readonly kind: "symbols";
    readonly symbols: readonly DocumentSymbol[];
  };
}
export type ReadKind = keyof Results;
export interface Observation<K extends ReadKind> {
  readonly apiVersion: 1;
  readonly resourceId: ResourceId;
  /** Original open's lower bound, NOT current text or position authority. */
  readonly openedVersion: readonly {
    readonly replicaId: number;
    readonly timestamp: number;
  }[];
  readonly result: Results[K];
}
export type Failure =
  | "http"
  | "protocol"
  | "transport"
  | "context_lost"
  | "cancelled"
  | "busy"
  | "state"
  | "capacity";
export class BufferClientError extends Error {
  constructor(readonly kind: Failure, readonly status?: number) {
    super(`Owned buffer unavailable (${kind})`);
    this.name = "BufferClientError";
  }
}

export function requireValue(value: unknown): asserts value {
  if (!value) throw new BufferClientError("protocol");
}
export function record(value: unknown, fields: readonly string[]) {
  requireValue(value && typeof value === "object" && !Array.isArray(value));
  const row = value as Record<string, unknown>;
  requireValue(
    Object.keys(row).length === fields.length &&
      fields.every((field) => Object.hasOwn(row, field)),
  );
  return row;
}
export function integer(value: unknown, min = 0, max = 0xffff_ffff): number {
  requireValue(typeof value === "number" && Number.isInteger(value));
  requireValue(value >= min && value <= max);
  return value;
}
export function text(value: unknown, max = 64 * 1024): string {
  requireValue(typeof value === "string");
  requireValue(
    value.length <= max && new TextEncoder().encode(value).length <= max,
  );
  return value;
}
export function list(value: unknown, limit: number): unknown[] {
  requireValue(Array.isArray(value) && value.length <= limit);
  return value;
}
export function decodeResourceId(value: unknown): ResourceId {
  requireValue(
    typeof value === "string" && /^[0-9a-f]{32}-[0-9a-f]{16}$/.test(value) &&
      !value.endsWith("-0000000000000000"),
  );
  return value as ResourceId;
}
export function decodeSnapshot(
  value: unknown,
  status: number,
  expected?: ResourceId,
): Snapshot {
  const row = record(value, ["apiVersion", "resourceId", "state", "pending"]);
  const id = decodeResourceId(row.resourceId);
  requireValue(row.apiVersion === 1 && (!expected || id === expected));
  requireValue(
    row.state === "prepared" || row.state === "open" ||
      row.state === "released" || row.state === "unknown",
  );
  requireValue(
    typeof row.pending === "boolean" &&
      (row.pending ? status === 202 : status === 200) &&
      !(row.state === "released" && row.pending),
  );
  return Object.freeze({
    apiVersion: 1,
    resourceId: id,
    state: row.state,
    pending: row.pending,
  });
}
