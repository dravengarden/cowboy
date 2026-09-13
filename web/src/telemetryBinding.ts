// Core Service contract, not Plugin UI IR or a serialized execution authority.
type Phase =
  | "prepared"
  | "dispatching"
  | "needs_attention"
  | "aborted"
  | "completed"
  | "rejected";
export type ResolutionAction =
  | "abort_before_dispatch"
  | "accept_applied"
  | "record_rejected";
type Attention =
  | "authorization_ended"
  | "uncertain"
  | "invalid_evidence"
  | "head_changed";
interface Installation {
  plugin_id: string;
  plugin_version: string;
  generation_digest: string;
  installation_revision: string;
  contract_fingerprint: string;
}
export interface BindingHead {
  revision: string;
  policy_epoch: string;
  selection: Installation | null;
}
type Change =
  | { kind: "select"; installation: Installation; policy_epoch: string }
  | { kind: "revoke"; policy_epoch: string }
  | {
    kind: "restore";
    forward_request_digest: string;
    selection: Installation | null;
    policy_epoch: string;
  };
export interface BindingOperation {
  operation_id: string;
  machine_id: string;
  operation_digest: string;
  phase: Phase;
  attention: Attention | null;
  expected: BindingHead | null;
  change: Change;
}
export interface ResolutionReceipt {
  schema: 1;
  resolution_id: string;
  operation_id: string;
  machine_id: string;
  action: ResolutionAction;
  operation_digest: string;
  phase: "aborted" | "completed" | "rejected";
  resolved_at_ms: number;
}
export interface ResolutionPlan {
  schema: 1;
  plan_id: string;
  action: ResolutionAction;
  expires_at_ms: number;
  confirmation_available: boolean;
  operation: BindingOperation;
  result_phase: "aborted" | "completed" | "rejected";
  result_head: BindingHead | null;
}
export interface BindingStatus {
  schema: 1;
  resolution_admission: "open" | "closed";
  journal: { state: "absent" } | {
    state: "retained";
    current: BindingHead | null;
    latest: BindingOperation;
    resolution: ResolutionReceipt | null;
  };
}

function invalid(): never {
  throw new Error("Invalid telemetry binding response");
}
function record(
  value: unknown,
  keys: readonly string[],
): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return invalid();
  }
  const entries = Object.entries(value);
  if (
    entries.length !== keys.length ||
    entries.some(([key]) => !keys.includes(key))
  ) return invalid();
  return Object.fromEntries(entries);
}
function choice<const T extends string>(
  value: unknown,
  choices: readonly T[],
): T {
  return choices.find((candidate) => value === candidate) ?? invalid();
}
function string(value: unknown, pattern: RegExp, max = 128): string {
  if (typeof value !== "string" || value.length > max || !pattern.test(value)) {
    return invalid();
  }
  return value;
}
function id(value: unknown, min = 1): string {
  const result = string(value, /^[A-Za-z0-9_-]+$/);
  return result.length >= min ? result : invalid();
}
function digest(value: unknown): string {
  return string(value, /^sha256:[0-9a-f]{64}$/);
}
function counter(value: unknown): string {
  const result = string(value, /^(0|[1-9][0-9]*)$/, 20);
  return BigInt(result) <= 18446744073709551615n ? result : invalid();
}
function timestamp(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0
    ? value
    : invalid();
}
function schema(value: unknown): 1 {
  return value === 1 ? value : invalid();
}
function boolean(value: unknown): boolean {
  return typeof value === "boolean" ? value : invalid();
}
function action(value: unknown): ResolutionAction {
  return choice(value, [
    "abort_before_dispatch",
    "accept_applied",
    "record_rejected",
  ]);
}
function terminal(
  value: unknown,
  selected: ResolutionAction,
): ResolutionReceipt["phase"] {
  const expected = {
    abort_before_dispatch: "aborted",
    accept_applied: "completed",
    record_rejected: "rejected",
  } as const;
  return value === expected[selected] ? expected[selected] : invalid();
}
function installation(value: unknown): Installation {
  const r = record(value, [
    "plugin_id",
    "plugin_version",
    "generation_digest",
    "installation_revision",
    "contract_fingerprint",
  ]);
  return {
    plugin_id: id(r.plugin_id),
    plugin_version: string(
      r.plugin_version,
      /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/,
    ),
    generation_digest: digest(r.generation_digest),
    installation_revision: string(
      r.installation_revision,
      /^installation-[0-9a-f]{64}$/,
    ),
    contract_fingerprint: digest(r.contract_fingerprint),
  };
}
function nullable<T>(value: unknown, parse: (value: unknown) => T): T | null {
  return value === null ? null : parse(value);
}
function head(value: unknown): BindingHead {
  const r = record(value, ["revision", "policy_epoch", "selection"]);
  const result = {
    revision: counter(r.revision),
    policy_epoch: counter(r.policy_epoch),
    selection: nullable(r.selection, installation),
  };
  if (
    (result.revision === "0" &&
      (result.policy_epoch !== "0" || result.selection !== null)) ||
    (result.selection !== null && result.policy_epoch === "0")
  ) return invalid();
  return result;
}
function change(value: unknown): Change {
  if (!value || typeof value !== "object" || !("kind" in value)) {
    return invalid();
  }
  switch (value.kind) {
    case "select": {
      const r = record(value, ["kind", "installation", "policy_epoch"]);
      return {
        kind: "select",
        installation: installation(r.installation),
        policy_epoch: counter(r.policy_epoch),
      };
    }
    case "revoke": {
      const r = record(value, ["kind", "policy_epoch"]);
      return { kind: "revoke", policy_epoch: counter(r.policy_epoch) };
    }
    case "restore": {
      const r = record(value, [
        "kind",
        "forward_request_digest",
        "selection",
        "policy_epoch",
      ]);
      return {
        kind: "restore",
        forward_request_digest: digest(r.forward_request_digest),
        selection: nullable(r.selection, installation),
        policy_epoch: counter(r.policy_epoch),
      };
    }
    default:
      return invalid();
  }
}
function operation(value: unknown): BindingOperation {
  const r = record(value, [
    "operation_id",
    "machine_id",
    "operation_digest",
    "phase",
    "attention",
    "expected",
    "change",
  ]);
  const phase = choice(r.phase, [
    "prepared",
    "dispatching",
    "needs_attention",
    "aborted",
    "completed",
    "rejected",
  ]);
  const attention = nullable(
    r.attention,
    (v) =>
      choice(v, [
        "authorization_ended",
        "uncertain",
        "invalid_evidence",
        "head_changed",
      ]),
  );
  if ((phase === "needs_attention") !== (attention !== null)) return invalid();
  return {
    operation_id: id(r.operation_id, 16),
    machine_id: id(r.machine_id),
    operation_digest: digest(r.operation_digest),
    phase,
    attention,
    expected: nullable(r.expected, head),
    change: change(r.change),
  };
}
export function parseResolutionReceipt(value: unknown): ResolutionReceipt {
  const r = record(value, [
    "schema",
    "resolution_id",
    "operation_id",
    "machine_id",
    "action",
    "operation_digest",
    "phase",
    "resolved_at_ms",
  ]);
  const selected = action(r.action);
  return {
    schema: schema(r.schema),
    resolution_id: id(r.resolution_id, 16),
    operation_id: id(r.operation_id, 16),
    machine_id: id(r.machine_id),
    action: selected,
    operation_digest: digest(r.operation_digest),
    phase: terminal(r.phase, selected),
    resolved_at_ms: timestamp(r.resolved_at_ms),
  };
}
export function parseResolutionPlan(value: unknown): ResolutionPlan {
  const r = record(value, [
    "schema",
    "plan_id",
    "action",
    "expires_at_ms",
    "confirmation_available",
    "operation",
    "result_phase",
    "result_head",
  ]);
  const selected = action(r.action);
  const before = operation(r.operation);
  if (
    selected === "abort_before_dispatch"
      ? before.phase !== "prepared"
      : !["dispatching", "needs_attention"].includes(before.phase)
  ) return invalid();
  return {
    schema: schema(r.schema),
    plan_id: id(r.plan_id, 16),
    action: selected,
    expires_at_ms: timestamp(r.expires_at_ms),
    confirmation_available: boolean(r.confirmation_available),
    operation: before,
    result_phase: terminal(r.result_phase, selected),
    result_head: nullable(r.result_head, head),
  };
}
export function parseBindingStatus(value: unknown): BindingStatus {
  const r = record(value, ["schema", "resolution_admission", "journal"]);
  const base = {
    schema: schema(r.schema),
    resolution_admission: choice(r.resolution_admission, ["open", "closed"]),
  };
  if (!r.journal || typeof r.journal !== "object" || !("state" in r.journal)) {
    return invalid();
  }
  if (r.journal.state === "absent") {
    record(r.journal, ["state"]);
    return { ...base, journal: { state: "absent" } };
  }
  if (r.journal.state !== "retained") return invalid();
  const j = record(r.journal, ["state", "current", "latest", "resolution"]);
  const latest = operation(j.latest);
  const resolution = nullable(j.resolution, parseResolutionReceipt);
  if (
    resolution &&
    (resolution.operation_id !== latest.operation_id ||
      resolution.machine_id !== latest.machine_id ||
      resolution.phase !== latest.phase)
  ) return invalid();
  return {
    ...base,
    journal: {
      state: "retained",
      current: nullable(j.current, head),
      latest,
      resolution,
    },
  };
}

export function matchesResolution(
  plan: ResolutionPlan,
  receipt: ResolutionReceipt,
): boolean {
  return receipt.resolution_id === plan.plan_id &&
    receipt.operation_id === plan.operation.operation_id &&
    receipt.machine_id === plan.operation.machine_id &&
    receipt.action === plan.action &&
    receipt.operation_digest === plan.operation.operation_digest &&
    receipt.phase === plan.result_phase;
}

// UX expiry is conservative and sticky. Only the server owns authority clocks.
export class PreviewDeadline {
  private expired = false;
  private highWater: number;
  private readonly end: number;
  constructor(
    private readonly expires: number,
    private readonly received: number,
    wall: number,
  ) {
    this.highWater = wall;
    this.end = received + Math.min(120_000, Math.max(0, expires - wall));
  }
  ended(monotonic: number, wall: number): boolean {
    this.expired ||= !Number.isFinite(monotonic) || !Number.isFinite(wall) ||
      wall <= 0 || monotonic < this.received || monotonic >= this.end ||
      wall < this.highWater || wall >= this.expires;
    this.highWater = Math.max(this.highWater, wall);
    return this.expired;
  }
}

export const resolutionLabels: Record<ResolutionAction, string> = {
  abort_before_dispatch: "Abort before dispatch",
  accept_applied: "Record the applied binding",
  record_rejected: "Record the rejected binding",
};

async function request<T>(
  path: string,
  parse: (value: unknown) => T,
  signal: AbortSignal,
  body?: unknown,
): Promise<T> {
  const response = await fetch(path, {
    method: body === undefined ? "GET" : "POST",
    credentials: "same-origin",
    cache: "no-store",
    signal,
    headers: {
      accept: "application/json",
      ...(body === undefined ? {} : { "content-type": "application/json" }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  if (
    !response.ok ||
    !response.headers.get("content-type")?.includes("application/json")
  ) {
    await response.body?.cancel();
    throw new Error(
      "Telemetry binding request did not return verified evidence",
    );
  }
  const reader = response.body?.getReader();
  if (!reader) return invalid();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 64 * 1024) {
        await reader.cancel();
        return invalid();
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
  return parse(
    JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)),
  );
}
function path(operationId: string, action: string): string {
  return `/api/telemetry/binding/operations/${
    encodeURIComponent(id(operationId, 16))
  }/${action}`;
}
export const telemetryBindingApi = {
  status: (signal: AbortSignal) =>
    request("/api/telemetry/binding", parseBindingStatus, signal),
  plan: async (operationId: string, signal: AbortSignal) => {
    const plan = await request(
      path(operationId, "resolution-plan"),
      parseResolutionPlan,
      signal,
      {},
    );
    if (plan.operation.operation_id !== operationId) return invalid();
    return plan;
  },
  confirm: async (plan: ResolutionPlan, signal: AbortSignal) => {
    const receipt = await request(
      path(plan.operation.operation_id, "resolve"),
      parseResolutionReceipt,
      signal,
      { plan_id: plan.plan_id, action: plan.action },
    );
    return matchesResolution(plan, receipt) ? receipt : invalid();
  },
  receipt: async (operationId: string, signal: AbortSignal) => {
    const receipt = await request(
      path(operationId, "resolution"),
      parseResolutionReceipt,
      signal,
    );
    return receipt.operation_id === operationId ? receipt : invalid();
  },
};

/** A timeout is ambiguous. Only an exact read receipt can conclude this attempt.
 * The owner supplies a new, cancelable read scope; logout/unmount can refuse it. */
export async function confirmResolutionOnce(
  plan: ResolutionPlan,
  signal: AbortSignal,
  inspectionSignal: () => AbortSignal,
): Promise<ResolutionReceipt | null> {
  try {
    return await telemetryBindingApi.confirm(plan, signal);
  } catch {
    try {
      const receipt = await telemetryBindingApi.receipt(
        plan.operation.operation_id,
        inspectionSignal(),
      );
      return matchesResolution(plan, receipt) ? receipt : null;
    } catch {
      return null;
    }
  }
}
