/** Keys of a `fresh_login_json_v1` native identity recipe. */
export const NATIVE_IDENTITY_KEYS = [
  "kind",
  "authority",
  "relative_path",
  "subject_path",
  "tenant_path",
  "subject_claim",
] as const;

/** A bounded JSON path; `*` selects the sole member of an object. */
export function nativeIdentityPath(path: unknown): boolean {
  return (
    Array.isArray(path) &&
    path.length > 0 &&
    path.length <= 16 &&
    path.every(
      (part) =>
        typeof part === "string" &&
        part.length > 0 &&
        new TextEncoder().encode(part).length <= 128 &&
        !/[\p{Cc}]/u.test(part),
    )
  );
}

/** Optional paths are absent, null, or bounded. */
export function optionalNativeIdentityPath(path: unknown): boolean {
  return path === undefined || path === null || nativeIdentityPath(path);
}

/** JWT claims and sole-member paths are Provider SDK 3.3.0 recipes. */
export function usesExtendedNativeIdentityPaths(identity: Record<string, unknown>): boolean {
  return (
    (identity.subject_claim !== undefined && identity.subject_claim !== null) ||
    [identity.subject_path, identity.tenant_path].some(
      (path) => Array.isArray(path) && path.includes("*"),
    )
  );
}

type CompareVersions = (left: string, right: string) => number;

/** An `owner_declared_v1` recipe names only its authority; Provider SDK 3.4.0. */
export function ownerDeclaredIdentity(
  identity: Record<string, unknown>,
  executor: Record<string, unknown>,
  sdkVersion: string,
  compare: CompareVersions,
  identifier: (value: string) => boolean,
): boolean {
  return (
    compare(sdkVersion, "3.4.0") >= 0 &&
    executor.kind === "command_v1" &&
    Object.keys(identity).every((key) => key === "kind" || key === "authority") &&
    typeof identity.authority === "string" &&
    identifier(identity.authority)
  );
}

/** Absent, or the closed `ipv4_only_v1` policy of Provider SDK 3.4.0. */
export function runtimeNetworkPolicy(
  network: unknown,
  sdkVersion: string,
  compare: CompareVersions,
): boolean {
  return network === undefined || (network === "ipv4_only_v1" && compare(sdkVersion, "3.4.0") >= 0);
}
