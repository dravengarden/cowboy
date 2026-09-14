/** Core installation evidence, never a resumable or replayable capability. */
const phases = [
  "prepared",
  "syncing_authentication",
  "installing",
  "machine_acknowledged",
  "completed",
  "authentication_pending",
  "aborted",
  "needs_attention",
] as const;
const problems = [
  "interrupted",
  "preconditions_changed",
  "authentication_sync_failed",
  "transport_not_sent",
  "machine_rejected",
  "unknown_machine_outcome",
  "storage_failure",
] as const;
export type InstallPhase = typeof phases[number];
export type InstallProblem = typeof problems[number];
type MachinePluginKind =
  | "agent_provider"
  | "code_intelligence"
  | "telemetry_backend";
export interface InstallEvidence {
  readonly operation_id: string;
  readonly phase: InstallPhase;
  readonly problem: InstallProblem | null;
  readonly attention_from: InstallPhase | null;
  readonly plugin_kind: MachinePluginKind;
  readonly plugin_version: string;
  readonly generation_digest: string;
  readonly created_at_ms: number;
  readonly updated_at_ms: number;
}
export interface InstallHistory {
  readonly schema: "dravengarden.cowboy.plugin-install-history/v1";
  readonly admission_enabled: boolean;
  readonly execution_authorized: false;
  readonly machine_receipt_available: false;
  readonly requires_reconciliation: boolean;
  readonly operations: readonly InstallEvidence[];
}
const operationId = /^[A-Za-z0-9_-]{16,128}$/;
const digest = /^sha256:[0-9a-f]{64}$/;
function invalid(): never {
  throw new Error(
    "Plugin installation evidence is unavailable or incompatible",
  );
}
function object(
  value: unknown,
  fields: readonly string[],
): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  const result = value as Record<string, unknown>;
  if (
    Object.keys(result).length !== fields.length ||
    fields.some((field) => !Object.hasOwn(result, field))
  ) invalid();
  return result;
}
function member<const T extends readonly string[]>(
  values: T,
  value: unknown,
): T[number] {
  if (typeof value !== "string" || !values.some((item) => item === value)) {
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
function time(value: unknown): number {
  if (
    typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0 ||
    value > 8_640_000_000_000_000
  ) invalid();
  return value;
}
function evidence(value: unknown): InstallEvidence {
  const row = object(value, [
    "operation_id",
    "phase",
    "problem",
    "attention_from",
    "plugin_kind",
    "plugin_version",
    "generation_digest",
    "created_at_ms",
    "updated_at_ms",
  ]);
  const phase = member(phases, row.phase);
  const problem = row.problem === null ? null : member(problems, row.problem);
  const attention = row.attention_from === null
    ? null
    : member(phases, row.attention_from);
  const id = text(row.operation_id, 128);
  const generation = text(row.generation_digest, 71);
  const created = time(row.created_at_ms);
  const updated = time(row.updated_at_ms);
  if (!operationId.test(id) || !digest.test(generation) || updated < created) {
    invalid();
  }
  if (phase === "needs_attention") {
    if (
      !problem || !attention ||
      ![
        "prepared",
        "syncing_authentication",
        "installing",
        "machine_acknowledged",
      ].includes(attention)
    ) invalid();
  } else if (attention !== null) invalid();
  if (phase === "aborted") {
    if (
      !problem ||
      ![
        "preconditions_changed",
        "authentication_sync_failed",
        "transport_not_sent",
      ].includes(problem)
    ) invalid();
  } else if (phase === "authentication_pending") {
    if (problem !== "authentication_sync_failed") invalid();
  } else if (phase !== "needs_attention" && problem !== null) invalid();
  return Object.freeze({
    operation_id: id,
    phase,
    problem,
    attention_from: attention,
    plugin_kind: member(
      ["agent_provider", "code_intelligence", "telemetry_backend"] as const,
      row.plugin_kind,
    ),
    plugin_version: text(row.plugin_version, 128),
    generation_digest: generation,
    created_at_ms: created,
    updated_at_ms: updated,
  });
}

export function decodeInstallHistory(value: unknown): InstallHistory {
  const row = object(value, [
    "schema",
    "admission_enabled",
    "execution_authorized",
    "machine_receipt_available",
    "requires_reconciliation",
    "operations",
  ]);
  if (
    row.schema !== "dravengarden.cowboy.plugin-install-history/v1" ||
    row.execution_authorized !== false ||
    row.machine_receipt_available !== false ||
    typeof row.admission_enabled !== "boolean" ||
    typeof row.requires_reconciliation !== "boolean" ||
    !Array.isArray(row.operations) || row.operations.length > 32
  ) invalid();
  const operations = row.operations.map(evidence);
  if (
    new Set(operations.map((op) => op.operation_id)).size !== operations.length
  ) invalid();
  return Object.freeze({
    schema: row.schema,
    admission_enabled: row.admission_enabled,
    execution_authorized: false,
    machine_receipt_available: false,
    requires_reconciliation: row.requires_reconciliation,
    operations: Object.freeze(operations),
  });
}

export async function loadInstallHistory(
  machine: string,
  plugin: string,
  signal?: AbortSignal,
): Promise<InstallHistory> {
  const response = await fetch(
    `/api/machines/${encodeURIComponent(machine)}/plugins/${
      encodeURIComponent(plugin)
    }/installation-operations`,
    {
      credentials: "same-origin",
      cache: "no-store",
      ...(signal ? { signal } : {}),
    },
  );
  if (
    !response.ok ||
    !response.headers.get("content-type")?.includes("application/json")
  ) {
    await response.body?.cancel();
    invalid();
  }
  const reader = response.body?.getReader();
  if (!reader) invalid();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 128 * 1024) {
        await reader.cancel();
        invalid();
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    return decodeInstallHistory(
      JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)),
    );
  } catch {
    return invalid();
  }
}

/** Allocate once for one explicit action. No retry loop or persisted grant. */
export function createPluginInstallRequest(
  version: string,
  artifactDigest: string,
) {
  if (!version || version.length > 128 || !digest.test(artifactDigest)) {
    throw new Error("Select an exact signed Plugin release");
  }
  return Object.freeze({
    operation_id: `installation-${crypto.randomUUID()}`,
    version,
    digest: artifactDigest,
  });
}
