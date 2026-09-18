import { type AuthStatus, type AuthStatusProbe, type ProductMe, productMeFromJson, type RegistrationPublicStatus } from "./authApi";
import { sameProductPrincipal } from "../productSyncIdentity";

export type AuthGateView = "loading" | "ready" | "login" | "activating" | "retry";

export interface AuthGateDecision {
  view: Exclude<AuthGateView, "loading">;
  me?: ProductMe;
  registration?: RegistrationPublicStatus;
  setup_required?: boolean;
  setup_pending?: boolean;
  /** The principal came from this device's last successful status probe,
   *  because Cowboy could not be reached (docs/offline-first-sync.md). */
  cached?: boolean;
}
const AUTH_STATUS_BACKOFF_MAX_MS = 15_000;

// --- Cached identity for offline boot --------------------------------------
// The cookie is the credential; `/api/auth/status` only reports it. When that
// report cannot be fetched, the last successful one lets the app mount from its
// local replica until the server says otherwise or a known deadline passes.
const AUTH_STATUS_CACHE_KEY = "cowboy:auth-status-cache";
const DEFAULT_IDLE_TIMEOUT_MS = 24 * 60 * 60 * 1000;

export interface CachedAuthStatus {
  readonly me: ProductMe;
  readonly capturedAt: number;
  /** `null` when the deployment runs without product authentication. */
  readonly expiresAt: number | null;
}

type StatusStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

function statusStorage(): StatusStorage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

/** The earliest server-side deadline after which a cached principal must not
 *  be trusted without contact: the sliding idle window, the primary-login
 *  maximum, and a required Passkey verification. */
export function authStatusCacheExpiry(body: AuthStatus, now: number): number | null {
  const me = body.me;
  if (!me) return now;
  if (me.auth_enabled === false) return null;
  const candidates = [now + (body.session?.idle_timeout_ms ?? DEFAULT_IDLE_TIMEOUT_MS)];
  if (typeof me.primary_reauth_due_at_ms === "number") candidates.push(me.primary_reauth_due_at_ms);
  if (me.passkey_reauth_required && typeof me.passkey_reauth_due_at_ms === "number") {
    candidates.push(me.passkey_reauth_due_at_ms);
  }
  return Math.min(...candidates);
}

export function rememberAuthStatus(
  body: AuthStatus,
  now = Date.now(),
  storage = statusStorage(),
): void {
  if (!storage) return;
  try {
    if (!body.me || typeof body.me.user_id !== "string") {
      storage.removeItem(AUTH_STATUS_CACHE_KEY);
      return;
    }
    const record: CachedAuthStatus = {
      me: body.me,
      capturedAt: now,
      expiresAt: authStatusCacheExpiry(body, now),
    };
    storage.setItem(AUTH_STATUS_CACHE_KEY, JSON.stringify(record));
  } catch {
    // Storage quota or privacy mode: the next probe simply has no cache.
  }
}

export function forgetAuthStatus(storage = statusStorage()): void {
  try {
    storage?.removeItem(AUTH_STATUS_CACHE_KEY);
  } catch {
    // nothing to forget
  }
}

export function readCachedAuthStatus(
  now = Date.now(),
  storage = statusStorage(),
): CachedAuthStatus | null {
  if (!storage) return null;
  try {
    const raw = storage.getItem(AUTH_STATUS_CACHE_KEY);
    if (raw === null) return null;
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== "object") return null;
    const record = parsed as Partial<CachedAuthStatus>;
    const me = productMeFromJson(record.me);
    if (
      !me || typeof me.user_id !== "string" ||
      typeof record.capturedAt !== "number" || !Number.isFinite(record.capturedAt) ||
      (record.expiresAt !== null && (typeof record.expiresAt !== "number" || !Number.isFinite(record.expiresAt)))
    ) return null;
    if (record.expiresAt !== null && record.expiresAt <= now) return null;
    return { me, capturedAt: record.capturedAt, expiresAt: record.expiresAt };
  } catch {
    return null;
  }
}

/** A mountable decision from the cache, or `null` when the device must wait
 *  for the server. Only a `retry` probe (network or 5xx) may use it: a login
 *  answer, a changed account, or an incompatible controller always wins. */
export function cachedAuthDecision(
  decision: AuthGateDecision,
  now = Date.now(),
  storage = statusStorage(),
): AuthGateDecision | null {
  if (decision.view !== "retry") return null;
  const cached = readCachedAuthStatus(now, storage);
  if (!cached) return null;
  return { view: "ready", me: cached.me, cached: true };
}

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
    return sameProductPrincipal(current, decision.me) ? "update" : "teardown";
  }
  if (decision.view === "login") return "teardown";
  return "stay";
}

export { announceProductSessionEnd, PRODUCT_SESSION_END_EVENT } from "../productSessionEnd";
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
