import type { MachineSummary, SessionMeta } from "./protocol";

/** Live revisions are monotonic for one Controller process. A connect resync
 * deliberately accepts a lower revision after a Controller restart. */
export function acceptsMachineSnapshot(
  currentRevision: number,
  loaded: boolean,
  nextRevision: number,
  resync: boolean,
): boolean {
  return resync || !loaded || nextRevision > currentRevision;
}

/** Reconcile the session-derived portion of a pushed Machine snapshot without
 * waiting for another Machine inventory event. Session lifecycle already rides
 * the same WebSocket, so capacity and component leases stay live for free. */
export function projectMachineOccupancy(
  machines: readonly MachineSummary[],
  sessions: readonly SessionMeta[],
): readonly MachineSummary[] {
  const sessionLoads = new Map<
    string,
    { active: number; providers: Map<string, number> }
  >();
  for (const session of sessions) {
    if (session.status === "exited" || !session.machine_id) continue;
    let load = sessionLoads.get(session.machine_id);
    if (!load) {
      load = { active: 0, providers: new Map() };
      sessionLoads.set(session.machine_id, load);
    }
    load.active += 1;
    load.providers.set(
      session.provider,
      (load.providers.get(session.provider) ?? 0) + 1,
    );
  }
  let changed = false;
  const projected = machines.map((machine) => {
    const load = sessionLoads.get(machine.id);
    const activeSessions = load?.active ?? 0;
    const schedulable = machine.connected && machine.workspaces.length > 0 &&
      !machine.capacity.draining &&
      activeSessions < machine.capacity.max_sessions;
    let componentsChanged = false;
    const components = machine.components.map((component) => {
      let activeLeases = component.active_leases;
      if (component.id.kind === "acp_runtime") {
        activeLeases = activeSessions;
      } else if (
        component.id.kind === "provider_adapter" ||
        component.id.kind === "provider_cli"
      ) {
        const slot = component.id.slot ?? "";
        activeLeases = load?.providers.get(slot) ?? 0;
        if (slot === "claude") {
          activeLeases += load?.providers.get("claude-code") ?? 0;
          activeLeases += load?.providers.get("claude-deepseek") ?? 0;
        }
      }
      if (activeLeases === component.active_leases) return component;
      componentsChanged = true;
      return { ...component, active_leases: activeLeases };
    });
    if (
      activeSessions === machine.active_sessions &&
      schedulable === machine.schedulable &&
      !componentsChanged
    ) return machine;
    changed = true;
    return {
      ...machine,
      active_sessions: activeSessions,
      schedulable,
      components,
    };
  });
  return changed ? projected : machines;
}
