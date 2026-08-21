import type { AuthStatusProbe, ProductMe, RegistrationPublicStatus } from "./authApi";

export type AuthGateView = "loading" | "ready" | "login" | "activating" | "retry";

export interface AuthGateDecision {
  view: Exclude<AuthGateView, "loading">;
  me?: ProductMe;
  registration?: RegistrationPublicStatus;
  setup_required?: boolean;
  setup_pending?: boolean;
}
const AUTH_STATUS_BACKOFF_MAX_MS = 15_000;

export function classifyAuthStatus(probe: AuthStatusProbe): AuthGateDecision {
  if (probe.kind === "ok") {
    const registration = probe.body.registration;
    if (probe.body.me) {
      return {
        view: "ready",
        me: probe.body.me,
        registration,
        setup_required: probe.body.setup_required === true,
        setup_pending: probe.body.setup_pending === true,
      };
    }
    return {
      view: "login",
      registration,
      setup_required: probe.body.setup_required === true,
      setup_pending: probe.body.setup_pending === true,
    };
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

export type ReadyStatusAction = "stay" | "update" | "teardown";

/** Once the apps are mounted, only 200 + missing/changed `me` tears them down.
 *  Network / 5xx / 404 / 501 must not unmount a ready session. */
export function nextReadyStatusAction(
  current: ProductMe,
  decision: AuthGateDecision,
): ReadyStatusAction {
  if (decision.view === "ready" && decision.me) {
    return decision.me.account === current.account ? "update" : "teardown";
  }
  if (decision.view === "login") return "teardown";
  return "stay";
}

export const PRODUCT_SESSION_END_EVENT = "cowboy:product-sign-out";
export const PRODUCT_AUTH_LOST_EVENT = "cowboy:product-auth-lost";
export const WS_AUTH_REQUIRED_CLOSE_CODE = 4001;

export type MeHandshake = "reconnect" | "logout" | "keep";

/** Handshake /me outcomes. Never treat a generic `!ok` as logout. */
export function classifyMeHandshake(
  status: number | "network",
): MeHandshake {
  if (status === 200) return "reconnect";
  if (status === 401 || status === 403) return "logout";
  return "keep";
}

export function isAuthLostCloseCode(code: number): boolean {
  return code === WS_AUTH_REQUIRED_CLOSE_CODE;
}

export function announceProductAuthLost(): void {
  globalThis.dispatchEvent(new Event(PRODUCT_AUTH_LOST_EVENT));
}

export function announceProductSessionEnd(): void {
  globalThis.dispatchEvent(new Event(PRODUCT_SESSION_END_EVENT));
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
