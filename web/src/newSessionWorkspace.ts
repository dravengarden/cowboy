export type NewSessionWorkspace = {
  value: string;
  label: string;
  help: string;
};

const COLUMBUS_PATH = "/home/draven/columbus";

/** Prefer the Columbus root for a fresh session, independent of API ordering. */
export function defaultNewSessionWorkspace<T extends NewSessionWorkspace>(
  workspaces: readonly T[],
): T | undefined {
  return workspaces.find((workspace) =>
    workspace.value.toLocaleLowerCase() === "columbus" ||
    workspace.label.toLocaleLowerCase() === "columbus" ||
    workspace.help.replace(/\/+$/, "") === COLUMBUS_PATH
  ) ?? workspaces[0];
}
