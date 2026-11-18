/** Closed Provider runtime capability vocabulary for manifests and validators. */
export const PROVIDER_CAPABILITIES = [
  "provider.runtime.v1",
  "provider.gateway.v1",
  "provider.execution-jsonrpc.v1",
  "provider.mcp-bindings.v1",
  "provider.managed-profiles.v1",
  "provider.managed-image.v1",
  "provider.native-custody.v1",
] as const;
export type ProviderCapability = (typeof PROVIDER_CAPABILITIES)[number];
export const providerCapabilities = new Set<string>(PROVIDER_CAPABILITIES);

/** Every capability is known and its Provider SDK floor is met. */
export function knownCapabilities(
  values: readonly string[],
  sdkVersion: string,
  compare: (left: string, right: string) => number,
): boolean {
  return values.every(
    (value) =>
      providerCapabilities.has(value) &&
      (value !== "provider.native-custody.v1" || compare(sdkVersion, "3.5.0") >= 0),
  );
}

/** The Provider supports this Cowboy's Controller and Machine contracts. */
export function validateProviderCompatibility(compatibility: Record<string, unknown>): void {
  if (
    typeof compatibility.min_controller_contract !== "number" ||
    typeof compatibility.max_controller_contract !== "number" ||
    compatibility.min_controller_contract > 2 ||
    compatibility.max_controller_contract < 2 ||
    typeof compatibility.min_machine_contract !== "number" ||
    typeof compatibility.max_machine_contract !== "number" ||
    compatibility.min_machine_contract > 4 ||
    compatibility.max_machine_contract < 4 ||
    typeof compatibility.ui_component_fingerprint !== "string" ||
    typeof compatibility.auth_contract_fingerprint !== "string"
  ) {
    throw new Error("Provider is incompatible with this Cowboy contract");
  }
}
