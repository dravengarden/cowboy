import type { SessionMeta } from "./protocol";
import type { ProviderUsage } from "./usageLimits";

interface Projection {
  account: string;
  pointer: string;
  target: string;
  numbers: string[];
  strings: string[];
}
let projections = new Map<string, Projection>();
const key = (provider: string, version: string, digest: string) => `${provider}/${version}/${digest}`;
function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}
function strings(value: unknown): value is string[] { return Array.isArray(value) && value.every((v) => typeof v === "string"); }

/** Only the exact signed host generation may interpret a session report. */
export function applySessionQuotaHosts(hosts: unknown): void {
  projections = new Map();
  if (!Array.isArray(hosts)) return;
  for (const hostValue of hosts) {
    const host = record(hostValue);
    const usage = record(host?.usage);
    const projection = record(usage?.session_rate_limits);
    if (!host || !projection || typeof host.id !== "string" || typeof host.plugin_version !== "string" ||
      typeof host.artifact_digest !== "string" || typeof usage?.account !== "string" ||
      typeof projection.pointer !== "string" || !projection.pointer.startsWith("/") || typeof projection.target !== "string" ||
      !strings(projection.required_number_fields ?? []) || !strings(projection.required_string_fields ?? [])) continue;
    projections.set(key(host.id, host.plugin_version, host.artifact_digest), {
      account: usage.account, pointer: projection.pointer, target: projection.target,
      numbers: (projection.required_number_fields ?? []) as string[],
      strings: (projection.required_string_fields ?? []) as string[],
    });
  }
}

/** Context tokens and cost are not quota. Never infer a missing plan window. */
export function sessionQuota(session: SessionMeta): ProviderUsage | undefined {
  if (!session.provider_version || !session.provider_generation_digest || !session.usage ||
    !Number.isFinite(session.usage.observed_at_ms) || session.usage.observed_at_ms <= 0) return undefined;
  const projection = projections.get(key(session.provider, session.provider_version, session.provider_generation_digest));
  if (!projection) return undefined;
  let value: unknown = session.usage.raw;
  for (const encoded of projection.pointer.slice(1).split("/")) {
    const field = encoded.replace(/~1/g, "/").replace(/~0/g, "~");
    const row = record(value);
    if (!row || !Object.hasOwn(row, field)) return undefined;
    value = row[field];
  }
  const limits = record(value);
  if (!limits || !projection.numbers.every((field) => typeof limits[field] === "number" && Number.isFinite(limits[field])) ||
    !projection.strings.every((field) => typeof limits[field] === "string")) return undefined;
  return {
    provider: projection.account, status: "session-only", source: "Session report",
    observed_at_ms: session.usage.observed_at_ms, rate_limits: { [projection.target]: limits },
  };
}
