import { useStore as usePreference } from "@cowboy/state-store";
import { defaultNewSessionProject } from "./newSessionProject";
import { newSessionProjectPreference } from "./newSessionProjectPreference";
import { useEffect, useMemo, useState } from "react";
import { projectAgentPluginInventory } from "@cowboy/provider-ui";
import type { MachineSummary } from "./protocol";
import { defaultNewSessionProvider } from "./newSessionProvider";
import type { FolderDefaults } from "./folderDefaults";
import type { FolderSessionHints } from "./folderSessionHints";
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
  defaults: FolderDefaults = {},
  hints?: FolderSessionHints,
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
  const folderProjects = projects.filter((project) =>
    (!defaults.machine || project.machineId === defaults.machine) &&
    (!defaults.project || project.projectId === defaults.project ||
      project.name === defaults.project)
  );
  // A directory's latest session fills only fields it leaves unset; `null`
  // still demands an explicit choice, and an unavailable hint falls back.
  const hintedProject = hints?.project &&
      !Object.hasOwn(defaults, "project") && !Object.hasOwn(defaults, "machine")
    ? projects.find((p) =>
      p.projectId === hints.project &&
      (!hints.machine || p.machineId === hints.machine)
    )
    : undefined;
  const defaultProject = defaults.project === null
    ? undefined
    : hintedProject ?? defaultNewSessionProject(
      folderProjects,
      machines,
      defaults.project || defaults.machine ? "" : preferredProject,
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
      // Creating a session prepares its workspace; a worker is admitted on
      // first use. A full worker pool must not disable creating a conversation.
      const full = false;
      const inventory = projectAgentPluginInventory(machine.plugins);
      const row = joinProviderInstallations(catalog?.providers ?? [], inventory)
        .find((row) => row.providerId === placement.provider);
      if (
        !row?.installedEntry || !row.installed
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
  const eligibleInstallations = installations.filter((i) =>
    !defaults.provider || i.provider === defaults.provider
  );
  const preferredRuntime = defaults.runtime_machine ??
    availability?.default_runtime_machine_id;
  const preferredInstallations = eligibleInstallations.filter((i) =>
    i.runtime_machine_id === preferredRuntime &&
    !i.full
  );
  const defaultCandidates = preferredRuntime
    ? preferredInstallations
    : eligibleInstallations.filter((i) => !i.full);
  const defaultProvider = defaultNewSessionProvider(
    defaultCandidates.map((i) => i.provider),
  );
  const explicitProvider = Object.hasOwn(defaults, "provider");
  const explicitRuntime = Object.hasOwn(defaults, "runtime_machine");
  const hintedInstallation = hints && !(explicitProvider && explicitRuntime)
    ? eligibleInstallations.find((i) =>
      !i.full &&
      i.runtime_machine_id ===
        (explicitRuntime ? preferredRuntime : hints.runtime_machine) &&
      i.provider ===
        (explicitProvider ? defaults.provider ?? defaultProvider : hints.provider)
    )
    : undefined;
  const installation = installationKey
    ? installations.find((i) => i.value === installationKey)
    : hintedInstallation ??
      defaultCandidates.find((i) => i.provider === defaultProvider);
  return {
    projects,
    project,
    defaultProjectValue: defaultProject?.value ?? "",
    /** Which current choices came from the directory's latest session. */
    hinted: {
      project: !projectKey && Boolean(hintedProject),
      installation: !installationKey && Boolean(hintedInstallation),
    },
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
