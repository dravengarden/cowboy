import {
  bindingCodec as c,
  type BindingHead,
  type BindingOperation,
  bindingPath,
  bindingRequest,
  type Installation,
} from "./telemetryBinding";

export interface BindingTarget {
  machine_id: string;
  installation: Installation;
}
export type BindingIntent =
  | { action: "select"; target: BindingTarget }
  | { action: "revoke" }
  | { action: "restore"; operation_id: string };
export interface BindingChoices {
  schema: 1;
  confirmation_available: boolean;
  owner_machine_id: string | null;
  targets: BindingTarget[];
  revoke_available: boolean;
  restore_operation_id: string | null;
}
export interface BindingPlan {
  schema: 1;
  plan_id: string;
  action: BindingIntent["action"];
  request_digest: string;
  expires_at_ms: number;
  confirmation_available: boolean;
  operation: BindingOperation;
  result_head: BindingHead;
  restores_operation_id: string | null;
}
export interface BindingReceipt {
  schema: 1;
  request_digest: string;
  operation: BindingOperation;
}

const same = (a: unknown, b: unknown) =>
  JSON.stringify(a) === JSON.stringify(b);

export function parseBindingChoices(value: unknown): BindingChoices {
  const r = c.record(value, [
    "schema",
    "confirmation_available",
    "owner_machine_id",
    "targets",
    "revoke_available",
    "restore_operation_id",
  ]);
  const owner = r.owner_machine_id === null ? null : c.id(r.owner_machine_id);
  if (!Array.isArray(r.targets) || r.targets.length > 64) return c.invalid();
  const targets = r.targets.map((value): BindingTarget => {
    const target = c.record(value, ["machine_id", "installation"]);
    const machine = c.id(target.machine_id);
    if (owner !== null && machine !== owner) return c.invalid();
    return {
      machine_id: machine,
      installation: c.installation(target.installation),
    };
  });
  if (
    new Set(targets.map((target) => JSON.stringify(target))).size !==
      targets.length
  ) return c.invalid();
  const revoke = c.boolean(r.revoke_available);
  const restore = r.restore_operation_id === null
    ? null
    : c.id(r.restore_operation_id, 16);
  if (owner === null && (revoke || restore !== null)) return c.invalid();
  return {
    schema: c.schema(r.schema),
    confirmation_available: c.boolean(r.confirmation_available),
    owner_machine_id: owner,
    targets,
    revoke_available: revoke,
    restore_operation_id: restore,
  };
}

export function parseBindingPlan(value: unknown): BindingPlan {
  const r = c.record(value, [
    "schema",
    "plan_id",
    "action",
    "request_digest",
    "expires_at_ms",
    "confirmation_available",
    "operation",
    "result_head",
    "restores_operation_id",
  ]);
  const operation = c.operation(r.operation);
  const action = c.choice(r.action, ["select", "revoke", "restore"]);
  const plan = c.id(r.plan_id, 16);
  const result = c.head(r.result_head);
  const restores = r.restores_operation_id === null
    ? null
    : c.id(r.restores_operation_id, 16);
  const before = operation.expected ??
    { revision: "0", policy_epoch: "0", selection: null };
  const change = operation.change;
  const selection = change.kind === "select"
    ? change.installation
    : change.kind === "restore"
    ? change.selection
    : null;
  if (
    operation.phase !== "prepared" || operation.operation_id !== plan ||
    change.kind !== action ||
    (action === "restore") !== (restores !== null) ||
    BigInt(result.revision) !== BigInt(before.revision) + 1n ||
    BigInt(result.policy_epoch) !== BigInt(before.policy_epoch) + 1n ||
    result.policy_epoch !== change.policy_epoch ||
    !same(result.selection, selection)
  ) return c.invalid();
  return {
    schema: c.schema(r.schema),
    plan_id: plan,
    action,
    request_digest: c.digest(r.request_digest),
    expires_at_ms: c.timestamp(r.expires_at_ms),
    confirmation_available: c.boolean(r.confirmation_available),
    operation,
    result_head: result,
    restores_operation_id: restores,
  };
}

export function parseBindingReceipt(value: unknown): BindingReceipt {
  const r = c.record(value, ["schema", "request_digest", "operation"]);
  return {
    schema: c.schema(r.schema),
    request_digest: c.digest(r.request_digest),
    operation: c.operation(r.operation),
  };
}

export function matchesBinding(
  plan: BindingPlan,
  receipt: BindingReceipt,
): boolean {
  const operation = receipt.operation;
  return receipt.request_digest === plan.request_digest &&
    operation.operation_id === plan.operation.operation_id &&
    operation.machine_id === plan.operation.machine_id &&
    same(operation.expected, plan.operation.expected) &&
    same(operation.change, plan.operation.change);
}

export const telemetryMutationApi = {
  choices: (signal: AbortSignal) =>
    bindingRequest(
      "/api/telemetry/binding/choices",
      parseBindingChoices,
      signal,
    ),
  plan: async (
    intent: BindingIntent,
    signal: AbortSignal,
  ): Promise<BindingPlan> => {
    const plan = await bindingRequest(
      "/api/telemetry/binding/plan",
      parseBindingPlan,
      signal,
      intent,
    );
    if (
      plan.action !== intent.action ||
      (intent.action === "select" &&
        (plan.operation.machine_id !== intent.target.machine_id ||
          !same(plan.result_head.selection, intent.target.installation))) ||
      (intent.action === "restore" &&
        plan.restores_operation_id !== intent.operation_id)
    ) return c.invalid();
    return plan;
  },
  confirm: async (
    plan: BindingPlan,
    signal: AbortSignal,
  ): Promise<BindingReceipt> => {
    const receipt = await bindingRequest(
      "/api/telemetry/binding/confirm",
      parseBindingReceipt,
      signal,
      { plan_id: plan.plan_id, action: plan.action },
    );
    return matchesBinding(plan, receipt) ? receipt : c.invalid();
  },
  receipt: async (
    plan: BindingPlan,
    signal: AbortSignal,
  ): Promise<BindingReceipt> => {
    const receipt = await bindingRequest(
      bindingPath(plan.operation.operation_id, "receipt"),
      parseBindingReceipt,
      signal,
    );
    return matchesBinding(plan, receipt) ? receipt : c.invalid();
  },
};

// One POST, then at most one independent GET. A retained Prepared/Dispatching
// or NeedsAttention row is evidence, not completion and never replay authority.
export async function confirmBindingOnce(
  plan: BindingPlan,
  signal: AbortSignal,
  inspectionSignal: () => AbortSignal,
): Promise<BindingReceipt | null> {
  try {
    return await telemetryMutationApi.confirm(plan, signal);
  } catch {
    try {
      return await telemetryMutationApi.receipt(plan, inspectionSignal());
    } catch {
      return null;
    }
  }
}
