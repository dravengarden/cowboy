export type MachineProviderComponent = {
  id: { kind: string; slot?: string };
  state: string;
  auth?: string;
  detail?: string;
};

export function machineProviderNeedsLogin(auth?: string): boolean {
  return auth === "signed_out";
}

export function machineProviderAuthLabel(_provider: string, auth?: string): string | null {
  if (!auth) return null;
  return auth.replaceAll("_", " ");
}

export function machineProviderAvailable(
  provider: string,
  components: readonly MachineProviderComponent[],
): boolean {
  const slot = provider === "claude-code" || provider === "claude-deepseek"
    ? "claude"
    : provider === "codex-deepseek"
    ? "codex"
    : provider;
  const cliReady = components.some((component) =>
    component.id.kind === "provider_cli" &&
    component.id.slot === slot &&
    component.state === "active" &&
    (provider === "codex-deepseek" || provider === "claude-deepseek" ||
      component.auth === "signed_in") &&
    (provider !== "gemini" || typeof component.detail === "string")
  );
  const isolatedBase = provider === "codex-deepseek"
    ? "codex"
    : provider === "claude-deepseek"
    ? "claude"
    : null;
  if (!cliReady || isolatedBase === null) return cliReady;
  const adapterActive = (adapterSlot: string): boolean => components.some((component) =>
      component.id.kind === "provider_adapter" &&
      component.id.slot === adapterSlot &&
      component.state === "active"
    );
  return adapterActive(isolatedBase) && adapterActive(provider);
}
