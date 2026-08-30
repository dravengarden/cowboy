/** Prefer standard Codex when the selected Machine can run it. */
export function defaultNewSessionProvider(
  availableProviderIds: readonly string[],
): string {
  return availableProviderIds.includes("codex")
    ? "codex"
    : availableProviderIds[0] ?? "";
}
