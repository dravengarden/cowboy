import type { SheetDockContext } from "@cowboy/app-shell";
import type { TaskSummary } from "./backgroundTasks";
import { callExecutionParameters, elapsedLabel, providerLabel } from "./managedCalls";
import { accountIdentity } from "./accountIdentity";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

export function taskDockContext(task: TaskSummary | null, now = Date.now(), accounts?: SubscriptionSnapshot): SheetDockContext {
  if (!task) return { title: "Background tasks" };
  const call = task.call;
  return {
    title: task.title,
    facts: call ? [
      ...callExecutionParameters(call).filter(fact => fact.value !== "Not recorded" && fact.label !== "Permission" && (fact.label !== "Preset" || !call.configuration?.model)),
      { label: "Account", value: call.provider_account_id ? accountIdentity(accounts, call.provider, call.provider_account_id).short : "Not recorded" },
      { label: "Provider", value: providerLabel(call.provider) },
      ...callExecutionParameters(call).filter(fact => fact.label === "Permission" && fact.value !== "Not recorded"),
    ] : task.subtitle ? [{ label: "Context", value: task.subtitle }] : undefined,
    status: task.group === "failed" ? "Failed" : task.group === "active" ? "Doing" : task.chip,
    meta: task.health ?? (call ? elapsedLabel(call.started_at_ms ?? call.created_at_ms, call.finished_at_ms ?? now) : undefined),
    tone: task.group === "failed" ? "error" : task.group === "needs_you" || task.health ? "warning" : task.group === "done" ? "success" : "neutral",
  };
}
