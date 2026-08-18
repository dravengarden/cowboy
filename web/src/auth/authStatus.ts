import type { AuthStatusProbe, ProductMe, RegistrationPublicStatus } from "./authApi";

export type AuthGateView = "loading" | "ready" | "login" | "activating" | "retry";

export interface AuthGateDecision {
  view: Exclude<AuthGateView, "loading">;
  me?: ProductMe;
  registration?: RegistrationPublicStatus;
}

const AUTH_STATUS_BACKOFF_MAX_MS = 15_000;

export function classifyAuthStatus(probe: AuthStatusProbe): AuthGateDecision {
  if (probe.kind === "ok") {
    const registration = probe.body.registration;
    if (probe.body.me) {
      return { view: "ready", me: probe.body.me, registration };
    }
    return { view: "login", registration };
  }
  if (probe.kind === "unsupported") {
    return { view: "activating" };
  }
  return { view: "retry" };
}

export function shouldMountProductApp(decision: AuthGateDecision): boolean {
  return decision.view === "ready" && decision.me != null;
}

export function shouldOpenWebSocket(decision: AuthGateDecision): boolean {
  return shouldMountProductApp(decision);
}

export function isLoginDecision(decision: AuthGateDecision): boolean {
  return decision.view === "login";
}

export function showRegistration(registration: RegistrationPublicStatus | undefined): boolean {
  return registration?.accepts_registration === true;
}

export function showRegistrationToken(
  registration: RegistrationPublicStatus | undefined,
): boolean {
  return showRegistration(registration) && registration?.mode === "token";
}

/** Same 1s, 2s, 4s, 8s, 15s cap as the connection banner. */
export function nextAuthStatusBackoffMs(attempts: number): number {
  return Math.min(
    AUTH_STATUS_BACKOFF_MAX_MS,
    1000 * 2 ** Math.max(0, attempts - 1),
  );
}

export function historyCacheName(version: string): string {
  return `${version}-history`;
}

export async function deleteProductHistoryCache(
  cachesApi: Pick<CacheStorage, "keys" | "delete"> | undefined = globalThis.caches,
): Promise<void> {
  if (!cachesApi) return;
  const keys = await cachesApi.keys();
  await Promise.all(
    keys
      .filter((key) => /^cowboy-v\d+-history$/.test(key))
      .map((key) => cachesApi.delete(key)),
  );
}


