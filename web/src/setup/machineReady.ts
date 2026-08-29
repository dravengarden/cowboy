export interface SetupMachine {
  id: string;
  display_name: string;
  local: boolean;
  schedulable: boolean;
  fingerprint?: string | null;
}

export function parseSetupMachines(value: unknown): SetupMachine[] {
  if (!Array.isArray(value)) return [];
  const machines: SetupMachine[] = [];
  for (const row of value) {
    if (row == null || typeof row !== "object") continue;
    const record = row as Record<string, unknown>;
    if (typeof record.id !== "string" || record.id.length === 0) continue;
    machines.push({
      id: record.id,
      display_name: typeof record.display_name === "string" ? record.display_name : record.id,
      local: record.local === true,
      schedulable: record.schedulable === true,
      fingerprint: typeof record.fingerprint === "string" ? record.fingerprint : null,
    });
  }
  return machines;
}

/** True when this instance has no enrolled device yet. A local stub or a
 *  not-yet-consumed enrollment slot is not enough to enter the session UI. */
export function needsMachineSetup(machines: readonly SetupMachine[]): boolean {
  return !machines.some((machine) =>
    machine.schedulable || (Boolean(machine.fingerprint) && !machine.local)
  );
}
