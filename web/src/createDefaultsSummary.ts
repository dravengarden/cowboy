import type { FolderDefaults } from "./folderDefaults";

/** One honest sentence about where the Create form's choices come from. */
export function createDefaultsSummary(
  { directoryName, explicitSources, hintSession }: {
    directoryName: string;
    /** Directory names that supplied explicit defaults (repeats allowed). */
    explicitSources: readonly string[];
    /** Title of the latest session whose choices fill the remaining fields. */
    hintSession?: string | undefined;
  },
): string {
  const owners = [...new Set(explicitSources)];
  const parts: string[] = [];
  if (owners.length) parts.push(`Defaults from ${owners.join(", ")}`);
  if (hintSession) {
    parts.push(
      `${owners.length ? "other choices" : "Choices"} match the latest session “${hintSession}”`,
    );
  }
  if (!parts.length) {
    return `${directoryName} has no defaults or sessions yet · using this device's defaults`;
  }
  return `${parts.join(" · ")} · Change any choice for this session.`;
}

type Chosen = {
  project: string;
  projectName: string;
  machine: string;
  runtime_machine: string;
  provider: string;
  preset?: string | undefined;
};

/** Explicit defaults that would make the current choices this directory's
 *  defaults, or undefined when its project and agent are already pinned to
 *  them. Offered so a directory is configured once, from a real choice. */
export function createDefaultsChanges(
  current: FolderDefaults,
  chosen: Chosen,
): FolderDefaults | undefined {
  const sameProject = current.project === chosen.project ||
    current.project === chosen.projectName;
  if (sameProject && current.provider === chosen.provider) return undefined;
  const changes: FolderDefaults = {};
  if (!sameProject) changes.project = chosen.project;
  if (!sameProject || (current.machine && current.machine !== chosen.machine)) {
    changes.machine = chosen.machine;
  }
  if (current.runtime_machine !== chosen.runtime_machine) {
    changes.runtime_machine = chosen.runtime_machine;
  }
  if (current.provider !== chosen.provider) changes.provider = chosen.provider;
  if (chosen.preset && current.preset !== chosen.preset) {
    changes.preset = chosen.preset;
  } else if (
    changes.provider && current.preset &&
    !current.preset.startsWith(`${chosen.provider}/`)
  ) {
    // Another agent's preset would make the saved defaults inconsistent.
    changes.preset = null;
  }
  return changes;
}
