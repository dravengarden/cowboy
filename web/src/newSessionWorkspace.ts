export type NewSessionWorkspace = {
  value: string;
  label: string;
  help: string;
};

/** Use the Machine-owned inventory order; Cowboy has no preferred repository. */
export function defaultNewSessionWorkspace<T extends NewSessionWorkspace>(
  workspaces: readonly T[],
): T | undefined {
  return workspaces[0];
}
