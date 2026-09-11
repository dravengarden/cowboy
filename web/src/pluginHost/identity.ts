/** Core's reader for the existing public host projection. Wire vocabulary is
 * not a registration API or native authority. */
export const HOST_SLOT_IDS = [
  "login.method",
  "account.panel",
  "provider.card",
  "provider.setup",
  "provider.settings",
  "provider.usage",
  "provider.empty",
  "code.intelligence",
] as const;
export type HostSlotId = typeof HOST_SLOT_IDS[number];
export type HostRendererId =
  | "login-password-v1"
  | "login-oidc-v1"
  | "account-passkeys-v1"
  | "provider-surface-v1"
  | "provider-usage-v1"
  | "provider-usage-activity-v1";

export function isPluginIdentifier(value: unknown): value is string {
  return typeof value === "string" && value.length <= 64 &&
    value.trim() === value &&
    /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value);
}
export function isPluginGeneration(value: unknown): value is string {
  return typeof value === "string" && value.length === 64 &&
    /^[a-f0-9]{64}$/.test(value);
}
export function isPluginArtifactDigest(value: unknown): value is string {
  return typeof value === "string" && value.length === 71 &&
    /^sha256:[a-f0-9]{64}$/.test(value);
}
export function isPluginVersion(value: unknown): value is string {
  return typeof value === "string" && value.length <= 64 &&
    value.trim() === value &&
    /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(value);
}
export function isHostSlotId(value: unknown): value is HostSlotId {
  return typeof value === "string" &&
    (HOST_SLOT_IDS as readonly string[]).includes(value);
}
export function rendererSupportsSlot(
  renderer: unknown,
  slot: HostSlotId,
): renderer is HostRendererId {
  switch (renderer) {
    case "login-password-v1":
    case "login-oidc-v1":
      return slot === "login.method";
    case "account-passkeys-v1":
      return slot === "account.panel";
    case "provider-surface-v1":
      return slot === "provider.card" || slot === "provider.setup" ||
        slot === "provider.settings" || slot === "provider.empty";
    case "provider-usage-v1":
    case "provider-usage-activity-v1":
      return slot === "provider.usage";
    default:
      return false;
  }
}

/** An incomplete exact identity never falls back to the default release. */
export type PluginHostRelease =
  | { readonly kind: "default" }
  | {
    readonly kind: "exact";
    readonly version: string;
    readonly artifactDigest: string;
  }
  | { readonly kind: "unavailable" };
export function pluginHostRelease(
  version: string | undefined,
  artifactDigest: string | undefined,
): PluginHostRelease {
  if (version === undefined && artifactDigest === undefined) {
    return { kind: "default" };
  }
  return isPluginVersion(version) && isPluginArtifactDigest(artifactDigest)
    ? { kind: "exact", version, artifactDigest }
    : { kind: "unavailable" };
}
