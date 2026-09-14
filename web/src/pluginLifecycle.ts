/** Core durable diagnostics, not current installation truth or effect authority. */
import {
  decodeInstallEvidence,
  type InstallEvidence,
} from "./pluginInstallation.ts";

const phases = [
  "prepared",
  "stopping_sessions",
  "uninstalling",
  "machine_uninstalled",
  "restoring_machine",
  "restoring_sessions",
  "completed",
  "compensated",
  "aborted",
  "needs_attention",
] as const;
const problems = [
  "interrupted",
  "preconditions_changed",
  "machine_unavailable",
  "machine_rejected",
  "unknown_machine_outcome",
  "storage_failure",
  "compensation_failed",
  "worker_recovery_unverified",
] as const;
export type UninstallPhase = typeof phases[number];
type UninstallProblem = typeof problems[number];
export interface UninstallEvidence {
  readonly evidence_schema: 1 | 2;
  readonly operation_id: string;
  readonly phase: UninstallPhase;
  readonly problem: UninstallProblem | null;
  readonly cause: UninstallProblem | null;
  readonly attention_from: UninstallPhase | null;
  readonly plugin_version: string;
  readonly generation_digest: string;
  readonly affected_session_count: number;
  readonly purge_after_ms: number;
  readonly created_at_ms: number;
  readonly updated_at_ms: number;
}
export interface ResolutionEvidence {
  readonly resolution_id: string;
  readonly action: "abort_before_effects";
  readonly resolved_at_ms: number;
  readonly plugin_mutation_performed: false;
  readonly session_mutation_performed: false;
  readonly worker_restoration_performed: false;
}
export type LifecycleEntry =
  | { readonly kind: "install"; readonly operation: InstallEvidence }
  | {
    readonly kind: "uninstall";
    readonly operation: UninstallEvidence;
    readonly resolution: ResolutionEvidence | null;
  };
export interface LifecycleHistory {
  readonly schema: "dravengarden.cowboy.plugin-lifecycle-history/v1";
  readonly machine_id: string;
  readonly plugin_id: string;
  readonly execution_authorized: false;
  readonly observation: "independent_durable_reads";
  readonly window: "latest_per_kind";
  readonly limit_per_kind: 32;
  readonly admission: {
    readonly install: boolean;
    readonly uninstall: boolean;
  };
  readonly requires_reconciliation: boolean;
  readonly entries: readonly LifecycleEntry[];
}
function invalid(): never {
  throw new Error("Plugin lifecycle evidence is unavailable or incompatible");
}
function object(
  value: unknown,
  keys: readonly string[],
): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  const row = value as Record<string, unknown>;
  if (
    Object.keys(row).length !== keys.length ||
    keys.some((key) => !Object.hasOwn(row, key))
  ) invalid();
  return row;
}
function member<const T extends readonly string[]>(
  items: T,
  value: unknown,
): T[number] {
  if (typeof value !== "string" || !items.some((item) => item === value)) {
    invalid();
  }
  return value as T[number];
}
function text(value: unknown, max: number): string {
  if (
    typeof value !== "string" || !value.length || value.length > max ||
    Array.from(value).some((character) => {
      const code = character.codePointAt(0)!;
      return code < 32 || (code >= 127 && code <= 159);
    })
  ) invalid();
  return value;
}
function id(value: unknown, minimum = 16): string {
  const result = text(value, 128);
  if (result.length < minimum || !/^[A-Za-z0-9_-]+$/.test(result)) invalid();
  return result;
}
function time(value: unknown): number {
  if (
    typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0 ||
    value > 8_640_000_000_000_000
  ) invalid();
  return value;
}
function uninstall(value: unknown): UninstallEvidence {
  const row = object(value, [
    "evidence_schema",
    "operation_id",
    "phase",
    "problem",
    "cause",
    "attention_from",
    "plugin_version",
    "generation_digest",
    "affected_session_count",
    "purge_after_ms",
    "created_at_ms",
    "updated_at_ms",
  ]);
  if (row.evidence_schema !== 1 && row.evidence_schema !== 2) invalid();
  const created = time(row.created_at_ms), updated = time(row.updated_at_ms);
  const generation = text(row.generation_digest, 71);
  if (
    updated < created || !/^sha256:[0-9a-f]{64}$/.test(generation) ||
    typeof row.affected_session_count !== "number" ||
    !Number.isInteger(row.affected_session_count) ||
    row.affected_session_count < 0 || row.affected_session_count > 1024
  ) invalid();
  return Object.freeze({
    evidence_schema: row.evidence_schema,
    operation_id: id(row.operation_id),
    phase: member(phases, row.phase),
    problem: row.problem === null ? null : member(problems, row.problem),
    cause: row.cause === null ? null : member(problems, row.cause),
    attention_from: row.attention_from === null
      ? null
      : member(phases, row.attention_from),
    plugin_version: text(row.plugin_version, 128),
    generation_digest: generation,
    affected_session_count: row.affected_session_count,
    purge_after_ms: time(row.purge_after_ms),
    created_at_ms: created,
    updated_at_ms: updated,
  });
}
function resolution(
  value: unknown,
  op: UninstallEvidence,
): ResolutionEvidence | null {
  if (value === null) return null;
  const row = object(value, [
    "resolution_id",
    "action",
    "resolved_at_ms",
    "plugin_mutation_performed",
    "session_mutation_performed",
    "worker_restoration_performed",
  ]);
  if (
    row.action !== "abort_before_effects" ||
    row.plugin_mutation_performed !== false ||
    row.session_mutation_performed !== false ||
    row.worker_restoration_performed !== false || op.phase !== "aborted" ||
    row.resolved_at_ms !== op.updated_at_ms ||
    op.attention_from !== "prepared" || op.cause !== null ||
    (op.problem !== "interrupted" && op.problem !== "storage_failure")
  ) invalid();
  return Object.freeze({
    resolution_id: id(row.resolution_id),
    action: "abort_before_effects",
    resolved_at_ms: time(row.resolved_at_ms),
    plugin_mutation_performed: false,
    session_mutation_performed: false,
    worker_restoration_performed: false,
  });
}
function entry(value: unknown): LifecycleEntry {
  if (!value || typeof value !== "object" || !("kind" in value)) invalid();
  if (value.kind === "install") {
    const row = object(value, ["kind", "operation"]);
    return Object.freeze({
      kind: "install",
      operation: decodeInstallEvidence(row.operation),
    });
  }
  if (value.kind === "uninstall") {
    const row = object(value, ["kind", "operation", "resolution"]);
    const operation = uninstall(row.operation);
    return Object.freeze({
      kind: "uninstall",
      operation,
      resolution: resolution(row.resolution, operation),
    });
  }
  return invalid();
}
export function lifecycleEntryKey(entry: LifecycleEntry): string {
  return `${entry.kind}:${entry.operation.operation_id}`;
}
export function decodeLifecycleHistory(
  value: unknown,
  machine: string,
  plugin: string,
): LifecycleHistory {
  const row = object(value, [
    "schema",
    "machine_id",
    "plugin_id",
    "execution_authorized",
    "observation",
    "window",
    "limit_per_kind",
    "admission",
    "requires_reconciliation",
    "entries",
  ]);
  const admission = object(row.admission, ["install", "uninstall"]);
  if (
    row.schema !== "dravengarden.cowboy.plugin-lifecycle-history/v1" ||
    row.machine_id !== id(machine, 1) || row.plugin_id !== id(plugin, 1) ||
    row.execution_authorized !== false ||
    row.observation !== "independent_durable_reads" ||
    row.window !== "latest_per_kind" || row.limit_per_kind !== 32 ||
    typeof admission.install !== "boolean" ||
    typeof admission.uninstall !== "boolean" ||
    typeof row.requires_reconciliation !== "boolean" ||
    !Array.isArray(row.entries) || row.entries.length > 64
  ) invalid();
  const entries = row.entries.map(entry);
  if (
    new Set(entries.map(lifecycleEntryKey)).size !== entries.length ||
    entries.filter((entry) => entry.kind === "install").length > 32 ||
    entries.filter((entry) => entry.kind === "uninstall").length > 32
  ) invalid();
  return Object.freeze({
    schema: row.schema,
    machine_id: machine,
    plugin_id: plugin,
    execution_authorized: false,
    observation: row.observation,
    window: row.window,
    limit_per_kind: 32,
    admission: Object.freeze({
      install: admission.install,
      uninstall: admission.uninstall,
    }),
    requires_reconciliation: row.requires_reconciliation,
    entries: Object.freeze(entries),
  });
}
export async function loadLifecycleHistory(
  machine: string,
  plugin: string,
  signal?: AbortSignal,
): Promise<LifecycleHistory> {
  id(machine, 1);
  id(plugin, 1);
  const response = await fetch(
    `/api/machines/${encodeURIComponent(machine)}/plugins/${
      encodeURIComponent(plugin)
    }/lifecycle-history`,
    {
      credentials: "same-origin",
      cache: "no-store",
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(8000)])
        : AbortSignal.timeout(8000),
    },
  );
  const reader = response.body?.getReader();
  if (!reader) invalid();
  try {
    if (
      !response.ok ||
      !response.headers.get("content-type")?.includes("application/json")
    ) invalid();
    const chunks: Uint8Array[] = [];
    let size = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 128 * 1024) invalid();
      chunks.push(value);
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return decodeLifecycleHistory(
      JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)),
      machine,
      plugin,
    );
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}
