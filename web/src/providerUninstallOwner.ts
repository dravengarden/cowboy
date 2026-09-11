import type { ProviderUiObservation } from "./providerUiOwner";
import {
  createProviderDialogOwner,
  expectProviderResponse,
} from "./providerDialogOwner";

export interface ProviderUninstallPlan {
  plan_id: string;
  machine_id: string;
  plugin_id: string;
  plugin_version: string;
  generation_digest: string;
  affected_sessions: { id: string; title: string; status: string }[];
  active_session_ids: string[];
  purge_after_ms: number;
  expires_at_ms: number;
  warning: string;
}
type UninstallDialog =
  | { phase: "preparing" }
  | { phase: "ready"; plan: ProviderUninstallPlan; confirmActive: boolean };

function validPlan(
  value: unknown,
  machine: string,
  plugin: string,
): value is ProviderUninstallPlan {
  if (typeof value !== "object" || value === null) return false;
  const plan = value as Record<string, unknown>;
  if (
    plan.machine_id !== machine || plan.plugin_id !== plugin ||
    !["plan_id", "plugin_version", "generation_digest", "warning"].every((
      key,
    ) => typeof plan[key] === "string") ||
    !plan.plan_id ||
    !["purge_after_ms", "expires_at_ms"].every((key) =>
      typeof plan[key] === "number" && Number.isSafeInteger(plan[key]) &&
      plan[key] > 0 && plan[key] <= 8.64e15
    ) ||
    !Array.isArray(plan.affected_sessions) ||
    plan.affected_sessions.length > 1024 ||
    !Array.isArray(plan.active_session_ids) ||
    plan.active_session_ids.length > 1024
  ) return false;
  const ids = new Set<string>();
  for (const session of plan.affected_sessions as unknown[]) {
    if (
      typeof session !== "object" || session === null || !("id" in session) ||
      typeof session.id !== "string" ||
      !session.id || ids.has(session.id) || !("title" in session) ||
      typeof session.title !== "string" ||
      !("status" in session) || typeof session.status !== "string"
    ) return false;
    ids.add(session.id);
  }
  return new Set(plan.active_session_ids).size ===
      plan.active_session_ids.length &&
    plan.active_session_ids.every((id: unknown) =>
      typeof id === "string" && ids.has(id)
    );
}

/** Owns the confirmation UI, not the admitted Service uninstall operation.
 * Each preview replaces only local observation; closing never sends undo or
 * cancels a confirmed request. The Service still checks actor/impact/expiry.
 */
export function createProviderUninstallOwner(
  fetch: (url: string, init?: RequestInit) => Promise<Response>,
  now: () => number = Date.now,
) {
  const dialog = createProviderDialogOwner<
    UninstallDialog,
    "prepare" | "confirm"
  >();
  return {
    snapshot: dialog.snapshot,
    subscribe: dialog.subscribe,
    lifecycle: dialog.lifecycle,
    close: () => dialog.close(),
    dispose: dialog.dispose,
    async prepare(
      machine: string,
      plugin: string,
      observation?: ProviderUiObservation,
    ): Promise<void> {
      if (observation?.active === false) return;
      const lease = dialog.open({ phase: "preparing" });
      if (!lease?.active) return;
      await lease.run(
        "prepare",
        "Could not prepare Provider uninstall",
        async () => {
          const response = await fetch(
            `/api/machines/${encodeURIComponent(machine)}/plugins/${
              encodeURIComponent(plugin)
            }/uninstall-plan`,
            { method: "POST" },
          );
          await expectProviderResponse(
            response,
            "Could not prepare Provider uninstall",
          );
          const plan: unknown = await response.json();
          if (!lease.active) return;
          if (observation?.active === false) {
            dialog.close(lease);
            return;
          }
          if (!validPlan(plan, machine, plugin)) {
            throw new Error(
              "Invalid Provider uninstall plan",
            );
          }
          // Copy only validated fields. The server remains the authority; this
          // snapshot prevents a changed response object from retargeting consent.
          lease.update(() => ({
            phase: "ready",
            plan: {
              plan_id: plan.plan_id,
              machine_id: plan.machine_id,
              plugin_id: plan.plugin_id,
              plugin_version: plan.plugin_version,
              generation_digest: plan.generation_digest,
              affected_sessions: plan.affected_sessions.map((
                { id, title, status },
              ) => ({ id, title, status })),
              active_session_ids: [...plan.active_session_ids],
              purge_after_ms: plan.purge_after_ms,
              expires_at_ms: plan.expires_at_ms,
              warning: plan.warning,
            },
            confirmActive: false,
          }));
        },
      );
    },
    setConfirmActive(confirmActive: boolean): void {
      if (dialog.snapshot().busy) return;
      dialog.current()?.update((value) =>
        value.phase === "ready" ? { ...value, confirmActive } : value
      );
    },
    async confirm(): Promise<void> {
      const lease = dialog.current();
      const value = lease?.value();
      if (!lease || value?.phase !== "ready" || dialog.snapshot().busy) return;
      const { plan, confirmActive } = value;
      if (plan.expires_at_ms <= now()) {
        lease.error(
          "This uninstall plan expired. Close it and prepare a new plan.",
        );
        return;
      }
      if (plan.active_session_ids.length && !confirmActive) {
        lease.error(
          "Confirm stopping the active sessions before uninstalling.",
        );
        return;
      }
      await lease.run("confirm", "Provider uninstall failed", async () => {
        const response = await fetch(
          `/api/machines/${encodeURIComponent(plan.machine_id)}/plugins/${
            encodeURIComponent(plan.plugin_id)
          }/uninstall`,
          {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
              plan_id: plan.plan_id,
              confirm_active_sessions: confirmActive,
            }),
          },
        );
        await expectProviderResponse(response, "Provider uninstall failed");
        dialog.close(lease);
      }).catch(() => {}); // The current dialog owns the visible failure.
    },
  };
}
export type ProviderUninstallOwner = ReturnType<
  typeof createProviderUninstallOwner
>;
