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
const machinePhases = [
  "prepared",
  "staging",
  "activating",
  "projecting_authentication",
] as const;
type MachineInstallPhase = typeof machinePhases[number];
export type MachineInstallOutcome =
  | { readonly state: "applied"; readonly revision: string }
  | {
    readonly state: "rejected";
    readonly reason: "expired" | "authorization_ended" | "target_changed";
  }
  | { readonly state: "pending"; readonly phase: MachineInstallPhase }
  | {
    readonly state: "unknown";
    readonly phase: MachineInstallPhase;
    readonly reason:
      | "interrupted"
      | "effect_failure"
      | "authorization_ended"
      | "expired";
  };
interface InstallEvidenceFields {
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
export type InstallEvidence =
  & InstallEvidenceFields
  & (
    | { readonly evidence_schema: 1; readonly machine_receipt: null }
    | {
      readonly evidence_schema: 2;
      readonly machine_receipt: MachineInstallOutcome | null;
    }
  );
export interface InstallHistory {
  readonly schema: "dravengarden.cowboy.plugin-install-history/v2";
  readonly admission_enabled: boolean;
  readonly execution_authorized: false;
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
function machineOutcome(value: unknown): MachineInstallOutcome | null {
  if (value === null) return null;
  if (!value || typeof value !== "object" || !("state" in value)) invalid();
  switch (value.state) {
    case "applied": {
      const row = object(value, ["state", "revision"]);
      const revision = text(row.revision, 77);
      if (!/^installation-[0-9a-f]{64}$/.test(revision)) invalid();
      return Object.freeze({ state: "applied", revision });
    }
    case "rejected": {
      const row = object(value, ["state", "reason"]);
      return Object.freeze({
        state: "rejected",
        reason: member(
          ["expired", "authorization_ended", "target_changed"] as const,
          row.reason,
        ),
      });
    }
    case "pending": {
      const row = object(value, ["state", "phase"]);
      return Object.freeze({
        state: "pending",
        phase: member(machinePhases, row.phase),
      });
    }
    case "unknown": {
      const row = object(value, ["state", "phase", "reason"]);
      return Object.freeze({
        state: "unknown",
        phase: member(machinePhases, row.phase),
        reason: member(
          [
            "interrupted",
            "effect_failure",
            "authorization_ended",
            "expired",
          ] as const,
          row.reason,
        ),
      });
    }
    default:
      return invalid();
  }
}

function evidence(value: unknown, legacy: boolean): InstallEvidence {
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
    ...(legacy ? [] : ["evidence_schema", "machine_receipt"]),
  ]);
  const schema = legacy ? 1 : row.evidence_schema;
  const receipt = legacy ? null : machineOutcome(row.machine_receipt);
  if (schema !== 1 && schema !== 2 || schema === 1 && receipt !== null) {
    invalid();
  }
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
        ...(schema === 2 && receipt?.state === "rejected"
          ? ["machine_rejected"]
          : []),
      ].includes(problem)
    ) invalid();
  } else if (phase === "authentication_pending") {
    if (problem !== "authentication_sync_failed") invalid();
  } else if (phase !== "needs_attention" && problem !== null) invalid();
  const fields: InstallEvidenceFields = {
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
  };
  if (schema === 1) {
    return Object.freeze({
      ...fields,
      evidence_schema: 1,
      machine_receipt: null,
    });
  }
  if (
    (receipt?.state === "pending" || receipt?.state === "unknown") &&
    receipt.phase === "projecting_authentication" &&
    fields.plugin_kind !== "agent_provider"
  ) invalid();
  switch (phase) {
    case "machine_acknowledged":
    case "completed":
    case "authentication_pending":
      if (receipt?.state !== "applied") invalid();
      break;
    case "prepared":
    case "syncing_authentication":
    case "installing":
      if (receipt !== null) invalid();
      break;
    case "aborted":
      if (
        receipt !== null &&
        (receipt.state !== "rejected" || problem !== "machine_rejected")
      ) invalid();
      break;
    case "needs_attention":
      if (receipt === null) {
        if (attention === "machine_acknowledged") invalid();
      } else if (receipt.state === "pending" || receipt.state === "unknown") {
        if (
          attention !== "installing" || problem !== "unknown_machine_outcome"
        ) invalid();
      } else if (receipt.state === "applied") {
        if (
          attention !== "machine_acknowledged" ||
          (problem !== "interrupted" && problem !== "storage_failure")
        ) invalid();
      } else invalid();
  }
  return Object.freeze({
    ...fields,
    evidence_schema: 2,
    machine_receipt: receipt,
  });
}

export function decodeInstallHistory(value: unknown): InstallHistory {
  const legacy = !!value && typeof value === "object" && "schema" in value &&
    value.schema === "dravengarden.cowboy.plugin-install-history/v1";
  const row = object(value, [
    "schema",
    "admission_enabled",
    "execution_authorized",
    ...(legacy ? ["machine_receipt_available"] : []),
    "requires_reconciliation",
    "operations",
  ]);
  if (
    (!legacy &&
      row.schema !== "dravengarden.cowboy.plugin-install-history/v2") ||
    row.execution_authorized !== false ||
    (legacy && row.machine_receipt_available !== false) ||
    typeof row.admission_enabled !== "boolean" ||
    typeof row.requires_reconciliation !== "boolean" ||
    !Array.isArray(row.operations) || row.operations.length > 32
  ) invalid();
  const operations = row.operations.map((operation) =>
    evidence(operation, legacy)
  );
  if (
    new Set(operations.map((op) => op.operation_id)).size !== operations.length
  ) invalid();
  return Object.freeze({
    schema: "dravengarden.cowboy.plugin-install-history/v2",
    admission_enabled: row.admission_enabled,
    execution_authorized: false,
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
