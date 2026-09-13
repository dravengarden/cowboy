import {
  bindingCodec as c,
  type BindingHead,
  type BindingOperation,
  bindingPath,
  bindingRequest,
} from "./telemetryBinding";

export type RecoveryAction = "reject_interrupted_prepared";
export interface RecoveryPlan {
  schema: 1;
  plan_id: string;
  action: RecoveryAction;
  request_digest: string;
  expires_at_ms: number;
  confirmation_available: boolean;
  operation: BindingOperation & { phase: "needs_attention" };
  machine_head: BindingHead;
}
export interface RecoveryReceipt {
  schema: 1;
  resolution_id: string;
  operation_id: string;
  machine_id: string;
  action: RecoveryAction;
  operation_digest: string;
  request_digest: string;
  resolved_at_ms: number;
}

export function parseRecoveryPlan(value: unknown): RecoveryPlan {
  const r = c.record(value, [
    "schema",
    "plan_id",
    "action",
    "request_digest",
    "expires_at_ms",
    "confirmation_available",
    "operation",
    "machine_head",
  ]);
  const before = c.operation(r.operation);
  if (before.phase !== "needs_attention") return c.invalid();
  const head = c.head(r.machine_head);
  if (
    JSON.stringify(head) !== JSON.stringify(
      before.expected ?? {
        revision: "0",
        policy_epoch: "0",
        selection: null,
      },
    )
  ) return c.invalid();
  return {
    schema: c.schema(r.schema),
    plan_id: c.id(r.plan_id, 16),
    action: c.choice(r.action, ["reject_interrupted_prepared"]),
    request_digest: c.digest(r.request_digest),
    expires_at_ms: c.timestamp(r.expires_at_ms),
    confirmation_available: c.boolean(r.confirmation_available),
    operation: { ...before, phase: "needs_attention" },
    machine_head: head,
  };
}

export function parseRecoveryReceipt(value: unknown): RecoveryReceipt {
  const r = c.record(value, [
    "schema",
    "resolution_id",
    "operation_id",
    "machine_id",
    "action",
    "operation_digest",
    "request_digest",
    "resolved_at_ms",
  ]);
  return {
    schema: c.schema(r.schema),
    resolution_id: c.id(r.resolution_id, 16),
    operation_id: c.id(r.operation_id, 16),
    machine_id: c.id(r.machine_id),
    action: c.choice(r.action, ["reject_interrupted_prepared"]),
    operation_digest: c.digest(r.operation_digest),
    request_digest: c.digest(r.request_digest),
    resolved_at_ms: c.timestamp(r.resolved_at_ms),
  };
}

export function matchesRecovery(
  plan: RecoveryPlan,
  receipt: RecoveryReceipt,
): boolean {
  return receipt.resolution_id === plan.plan_id &&
    receipt.action === plan.action &&
    receipt.operation_id === plan.operation.operation_id &&
    receipt.machine_id === plan.operation.machine_id &&
    receipt.operation_digest === plan.operation.operation_digest &&
    receipt.request_digest === plan.request_digest &&
    receipt.resolved_at_ms < plan.expires_at_ms;
}

export const telemetryRecoveryApi = {
  plan: async (
    operationId: string,
    signal: AbortSignal,
  ): Promise<RecoveryPlan> => {
    const plan = await bindingRequest(
      bindingPath(operationId, "machine-recovery-plan"),
      parseRecoveryPlan,
      signal,
      {},
    );
    return plan.operation.operation_id === operationId ? plan : c.invalid();
  },
  confirm: async (
    plan: RecoveryPlan,
    signal: AbortSignal,
  ): Promise<RecoveryReceipt> => {
    const receipt = await bindingRequest(
      bindingPath(plan.operation.operation_id, "recover-machine"),
      parseRecoveryReceipt,
      signal,
      {
        plan_id: plan.plan_id,
        action: plan.action,
      },
    );
    return matchesRecovery(plan, receipt) ? receipt : c.invalid();
  },
  receipt: async (
    plan: RecoveryPlan,
    signal: AbortSignal,
  ): Promise<RecoveryReceipt> => {
    const receipt = await bindingRequest(
      bindingPath(
        plan.operation.operation_id,
        `machine-recoveries/${encodeURIComponent(c.id(plan.plan_id, 16))}`,
      ),
      parseRecoveryReceipt,
      signal,
    );
    return matchesRecovery(plan, receipt) ? receipt : c.invalid();
  },
};

// An ended view may refuse the independent read. Never repeats a POST, chains
// Service resolution, or treats a missing/expired query handle as success.
export async function confirmRecoveryOnce(
  plan: RecoveryPlan,
  signal: AbortSignal,
  inspectionSignal: () => AbortSignal,
): Promise<RecoveryReceipt | null> {
  try {
    return await telemetryRecoveryApi.confirm(plan, signal);
  } catch {
    try {
      return await telemetryRecoveryApi.receipt(plan, inspectionSignal());
    } catch {
      return null;
    }
  }
}
