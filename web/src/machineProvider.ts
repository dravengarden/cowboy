export type MachineProviderComponent = {
  id: { kind: string; slot?: string };
  state: string;
  auth?: string;
  detail?: string;
};

export function machineProviderAvailable(
  provider: string,
  components: readonly MachineProviderComponent[],
): boolean {
  const slot = provider === "claude-code"
    ? "claude"
    : provider === "codex-deepseek"
    ? "codex"
    : provider;
  const cliReady = components.some((component) =>
    component.id.kind === "provider_cli" &&
    component.id.slot === slot &&
    component.state === "active" &&
    (provider === "codex-deepseek" || component.auth === "signed_in") &&
    (provider !== "gemini" || typeof component.detail === "string")
  );
  if (!cliReady || provider !== "codex-deepseek") return cliReady;
  const adapterActive = (adapterSlot: string): boolean => components.some((component) =>
      component.id.kind === "provider_adapter" &&
      component.id.slot === adapterSlot &&
      component.state === "active"
    );
  return adapterActive("codex") && adapterActive("codex-deepseek");
}
