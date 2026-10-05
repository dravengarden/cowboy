import { useStore as usePreference } from "@cowboy/state-store";
import { defaultNewSessionProject } from "./newSessionProject";
import { newSessionProjectPreference } from "./newSessionProjectPreference";
import { useEffect, useMemo, useState } from "react";
import { projectAgentPluginInventory } from "@cowboy/provider-ui";
import type { MachineSummary } from "./protocol";
import { defaultNewSessionProvider } from "./newSessionProvider";
import {
  joinProviderInstallations,
  useProviderCatalog,
} from "./providerCatalog";
import {
  projectChoices,
  projectJson,
  type ProjectPolicies,
} from "./projectPlacement";

interface Placement {
  runtime_machine_id: string;
  provider: string;
  mode: "local" | "remote";
}
interface Placements {
  machine_id: string;
  default_runtime_machine_id: string | null;
  placements: Placement[];
}

export function useProjectPlacement(
  open: boolean,
  machines: readonly MachineSummary[],
) {
  const [policies, setPolicies] = useState<ProjectPolicies>();
  const [projectKey, setProjectKey] = useState("");
  const [installationKey, setInstallationKey] = useState("");
  const [availability, setAvailability] = useState<Placements>();
  const [error, setError] = useState("");
  const { catalog, error: catalogError } = useProviderCatalog(open);
  const projects = useMemo(
    () => policies ? projectChoices(machines, policies) : [],
    [machines, policies],
  );
  const preferredProject = usePreference(newSessionProjectPreference);
  const defaultProject = defaultNewSessionProject(
    projects,
    machines,
    preferredProject,
  );
  const project = projectKey
    ? projects.find((p) => p.value === projectKey)
    : defaultProject;
  const machineId = project?.machineId ?? "";
  // Inventory changes revalidate readiness without resetting the user's project.
  const inventoryRevision = JSON.stringify(
    machines.map((m) => [m.id, m.connected, m.schedulable, m.plugins]),
  );
  useEffect(() => {
    if (!open) return;
    const controller = new AbortController();
    setError("");
    setPolicies(undefined);
    setAvailability(undefined);
    setProjectKey("");
    setInstallationKey("");
    void projectJson<ProjectPolicies>("/api/project-policies", {
      signal: controller.signal,
    })
      .then((value) => {
        if (!controller.signal.aborted) setPolicies(value);
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setError(String(error));
      });
    return (): void => controller.abort();
  }, [open]);
  useEffect(() => {
    if (!open || !machineId || !policies) return;
    const controller = new AbortController();
    setAvailability(undefined);
    setError("");
    void projectJson<Placements>(
      `/api/project-placements?${new URLSearchParams({
        machine_id: machineId,
      })}`,
      { signal: controller.signal },
    )
      .then((value) => {
        if (!controller.signal.aborted) setAvailability(value);
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setError(String(error));
      });
    return (): void => controller.abort();
  }, [open, machineId, policies, inventoryRevision]);
  const installations = useMemo(() => {
    if (availability?.machine_id !== machineId) return [];
    return availability.placements.flatMap((placement) => {
      const machine = machines.find((m) =>
        m.id === placement.runtime_machine_id
      );
      if (!machine?.connected || machine.capacity.draining) return [];
      // A connected, non-draining Machine that is not schedulable has reached
      // its live-session capacity. Show it as full instead of silently hiding
      // every installation it hosts.
      const full = !machine.schedulable;
      const inventory = projectAgentPluginInventory(machine.plugins);
      const row = joinProviderInstallations(catalog?.providers ?? [], inventory)
        .find((row) => row.providerId === placement.provider);
      if (
        !row?.installedEntry || !row.installed ||
        (row.installedEntry.manifest.authentication.required &&
          row.installed.materialization_state !== "current")
      ) return [];
      return [{
        value: JSON.stringify([machine.id, placement.provider]),
        label:
          `${row.installedEntry.manifest.display.name} · ${machine.display_name}`,
        machine,
        entry: row.installedEntry,
        installed: row.installed,
        full,
        ...placement,
      }];
    });
  }, [availability, machineId, machines, catalog]);
  const preferredInstallations = installations.filter((i) =>
    i.runtime_machine_id === availability?.default_runtime_machine_id &&
    !i.full
  );
  const defaultCandidates = availability?.default_runtime_machine_id
    ? preferredInstallations
    : installations.filter((i) => !i.full);
  const defaultProvider = defaultNewSessionProvider(
    defaultCandidates.map((i) => i.provider),
  );
  const installation = installationKey
    ? installations.find((i) => i.value === installationKey)
    : defaultCandidates.find((i) => i.provider === defaultProvider);
  return {
    projects,
    project,
    defaultProjectValue: defaultProject?.value ?? "",
    configuredDefaultProject: preferredProject,
    setDefaultProject: (value: string): void =>
      newSessionProjectPreference.set(value),
    installations,
    installation,
    selectProject: (value: string): void => {
      setProjectKey(value);
      setInstallationKey("");
    },
    selectInstallation: setInstallationKey,
    machineId,
    runtimeMachineId: installation?.runtime_machine_id ?? "",
    separate: installation?.mode === "remote",
    ready: Boolean(
      project && installation && !installation.full &&
        availability?.machine_id === machineId && !error && !catalogError,
    ),
    loading: Boolean(
      open &&
        (!policies || (machineId && availability?.machine_id !== machineId)),
    ),
    error: error || catalogError,
  };
}
