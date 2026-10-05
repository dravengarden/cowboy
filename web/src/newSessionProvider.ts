/** Standard Providers a new session prefers, in order. */
const PREFERRED_NEW_SESSION_PROVIDERS = ["claude-code", "codex"] as const;

/** Prefer standard Claude Code, then standard Codex, when the selected Machine
 *  can run it. */
export function defaultNewSessionProvider(
  availableProviderIds: readonly string[],
): string {
  return PREFERRED_NEW_SESSION_PROVIDERS.find((id) => availableProviderIds.includes(id)) ??
    availableProviderIds[0] ?? "";
}
