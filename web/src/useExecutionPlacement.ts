import { useEffect, useState } from "react";

interface Availability {
  enabled: boolean;
  default_runtime_machine_id?: string;
  runtime_machine_id?: string;
  machine_id?: string;
  providers?: string[];
}

/** Placement is server configuration, never inferred from a project name. */
export function useExecutionPlacement(
  open: boolean,
  machineId: string,
  inventoryRevision: string,
): {
  enabled: boolean;
  ready: boolean;
  error: string;
  runtimeMachineId: string;
  setRuntimeMachineId: (id: string) => void;
  separate: boolean;
  providers: readonly string[];
} {
  const [configuration, setConfiguration] = useState<Availability>();
  const [selection, setSelection] = useState<string>();
  const [availability, setAvailability] = useState<Availability>();
  const [error, setError] = useState("");
  const runtimeMachineId = selection ??
    configuration?.default_runtime_machine_id ?? machineId;
  const separate = configuration?.enabled === true &&
    runtimeMachineId !== machineId;
  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    setConfiguration(undefined);
    setSelection(undefined);
    setError("");
    void fetch("/api/execution-environments", {
      signal: controller.signal,
      cache: "no-store",
    })
      .then(async (response): Promise<Availability> => {
        if (response.status === 404) return { enabled: false };
        if (!response.ok) {
          throw new Error("Could not load session placement settings");
        }
        const value = await response.json() as Availability;
        if (typeof value.enabled !== "boolean") {
          throw new Error("Invalid session placement settings");
        }
        return value;
      }).then(setConfiguration).catch((error: unknown) => {
        if (!controller.signal.aborted) setError(String(error));
      });
    return (): void => controller.abort();
  }, [open]);
  useEffect(() => {
    setAvailability(undefined);
    if (!open || !separate || !machineId || !runtimeMachineId) return;
    const controller = new AbortController();
    const query = new URLSearchParams({
      machine_id: machineId,
      runtime_machine_id: runtimeMachineId,
    });
    setError("");
    void fetch(`/api/execution-environments?${query}`, {
      signal: controller.signal,
      cache: "no-store",
    })
      .then(async (response): Promise<Availability> => {
        if (!response.ok) {
          throw new Error("Could not check execution environment readiness");
        }
        return await response.json() as Availability;
      }).then((value) => {
        if (!controller.signal.aborted) setAvailability(value);
      }).catch((error: unknown) => {
        if (!controller.signal.aborted) setError(String(error));
      });
    return (): void => controller.abort();
  }, [open, separate, machineId, runtimeMachineId, inventoryRevision]);
  const matched = availability?.enabled === true &&
    availability.runtime_machine_id === runtimeMachineId &&
    availability.machine_id === machineId &&
    Array.isArray(availability.providers);
  return {
    enabled: configuration?.enabled === true,
    ready: Boolean(configuration && !error && (!separate || matched)),
    error,
    runtimeMachineId,
    setRuntimeMachineId: setSelection,
    separate,
    providers: matched ? availability.providers! : [],
  };
}
