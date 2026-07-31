export type MachineProviderComponent = {
  id: { kind: string; slot?: string };
  state: string;
  auth?: string;
};

export function machineProviderAvailable(
  provider: string,
  components: readonly MachineProviderComponent[],
): boolean {
  const slot = provider === "claude-code" ? "claude" : provider;
  return components.some((component) =>
    component.id.kind === "provider_cli" &&
    component.id.slot === slot &&
    component.state === "active" &&
    component.auth === "signed_in"
  );
}
