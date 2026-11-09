/** Closed Provider runtime capability vocabulary for manifests and validators. */
export const PROVIDER_CAPABILITIES = [
  "provider.runtime.v1",
  "provider.gateway.v1",
  "provider.execution-jsonrpc.v1",
  "provider.mcp-bindings.v1",
  "provider.managed-profiles.v1",
  "provider.managed-image.v1",
] as const;
export type ProviderCapability = (typeof PROVIDER_CAPABILITIES)[number];
export const providerCapabilities = new Set<string>(PROVIDER_CAPABILITIES);
