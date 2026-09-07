import {
  AuthApiError,
  type DeviceAuthorizationInfo,
  type DeviceAuthorizationRequest,
} from "./authApi";

export const DEVICE_AUTH_STORAGE_KEY = "cowboy:pending-device-authorization";
type RequestStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

function validCapability(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_-]{20,128}$/u.test(value);
}

export function parseDeviceAuthorization(
  value: unknown,
): DeviceAuthorizationRequest | null {
  if (value === null || typeof value !== "object") return null;
  const request = value as Partial<DeviceAuthorizationRequest>;
  return validCapability(request.request_id) &&
      validCapability(request.approval_token)
    ? { request_id: request.request_id, approval_token: request.approval_token }
    : null;
}

export function storedDeviceAuthorization(
  storage: RequestStorage = globalThis.sessionStorage,
): DeviceAuthorizationRequest | null {
  try {
    return parseDeviceAuthorization(
      JSON.parse(storage.getItem(DEVICE_AUTH_STORAGE_KEY) ?? "null"),
    );
  } catch {
    return null;
  }
}

export function sameDeviceAuthorization(
  left: DeviceAuthorizationRequest | null,
  right: DeviceAuthorizationRequest | null,
): boolean {
  return left?.request_id === right?.request_id &&
    left?.approval_token === right?.approval_token;
}

export function clearDeviceAuthorization(
  request: DeviceAuthorizationRequest,
  storage: RequestStorage = globalThis.sessionStorage,
): void {
  // A late response from the previous link must not consume a newer request.
  if (sameDeviceAuthorization(storedDeviceAuthorization(storage), request)) {
    storage.removeItem(DEVICE_AUTH_STORAGE_KEY);
  }
}

export function captureDeviceAuthorizationFromLocation(
  location: Pick<Location, "pathname" | "hash"> = globalThis.location,
  storage: RequestStorage = globalThis.sessionStorage,
  history: Pick<History, "replaceState"> = globalThis.history,
): boolean {
  if (location.pathname === "/auth/device" && location.hash) {
    const values = new URLSearchParams(location.hash.slice(1));
    const request = parseDeviceAuthorization({
      request_id: values.get("request_id"),
      approval_token: values.get("approval_token"),
    });
    if (request) {
      storage.setItem(DEVICE_AUTH_STORAGE_KEY, JSON.stringify(request));
    } else storage.removeItem(DEVICE_AUTH_STORAGE_KEY);
    history.replaceState(null, "", "/auth/device");
  }
  return storedDeviceAuthorization(storage) !== null ||
    location.pathname === "/auth/device";
}

type Phase =
  | "loading"
  | "pending"
  | "approved"
  | "denied"
  | "missing"
  | "expired"
  | "unavailable";
export interface DeviceAuthorizationState {
  phase: Phase;
  info: DeviceAuthorizationInfo | null;
  remainingMs: number;
  busy: boolean;
  error: string | null;
}

interface AuthorizationDependencies {
  inspect: (
    request: DeviceAuthorizationRequest,
    signal: AbortSignal,
  ) => Promise<DeviceAuthorizationInfo>;
  approve: (request: DeviceAuthorizationRequest) => Promise<unknown>;
  deny: (request: DeviceAuthorizationRequest) => Promise<unknown>;
  authorize: (operation: () => Promise<unknown>) => Promise<unknown>;
  clear: (request: DeviceAuthorizationRequest) => void;
  now?: () => number;
}

function unavailableError(reason: unknown): boolean {
  return reason instanceof AuthApiError &&
    [400, 410, 422].includes(reason.status);
}

function retryMessage(reason: unknown): string {
  if (reason instanceof DOMException && reason.name === "AbortError") {
    return "Verification was cancelled. You can try again while this request is still valid.";
  }
  if (reason instanceof AuthApiError && reason.status === 429) {
    return "Too many requests. Wait a moment, then try again.";
  }
  if (
    reason instanceof AuthApiError && [401, 403, 428].includes(reason.status)
  ) {
    return "Sign in to Cowboy again, then check this request before authorizing it.";
  }
  return "Could not check this authorization request. Check your connection and try again.";
}

/** One link's lifecycle, including stale-response fencing and elapsed-time checks. */
export class DeviceAuthorizationFlow {
  private state: DeviceAuthorizationState;
  private listeners = new Set<() => void>();
  private operation = 0;
  private inspection: AbortController | undefined;

  constructor(
    private readonly request: DeviceAuthorizationRequest | null,
    private readonly dependencies: AuthorizationDependencies,
  ) {
    this.state = {
      phase: request ? "loading" : "missing",
      info: null,
      remainingMs: 0,
      busy: false,
      error: null,
    };
  }

  getSnapshot = (): DeviceAuthorizationState => this.state;
  subscribe = (listener: () => void): () => void => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private update(patch: Partial<DeviceAuthorizationState>): void {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }

  private now(): number {
    return this.dependencies.now?.() ?? Date.now();
  }

  private finish(
    phase: "approved" | "denied" | "expired" | "unavailable",
  ): void {
    if (this.request) this.dependencies.clear(this.request);
    this.update({ phase, busy: false, error: null, remainingMs: 0 });
  }

  stop = (): void => {
    this.operation++;
    this.inspection?.abort();
  };

  inspect = async (): Promise<void> => {
    if (
      !this.request || this.state.busy ||
      !["loading", "pending"].includes(this.state.phase)
    ) return;
    this.stop();
    const operation = this.operation;
    const abort = new AbortController();
    this.inspection = abort;
    this.update({ phase: "loading", info: null, error: null, remainingMs: 0 });
    try {
      const info = await this.dependencies.inspect(this.request, abort.signal);
      if (operation !== this.operation) return;
      if (
        info.request_id !== this.request.request_id ||
        !Number.isSafeInteger(info.expires_at_ms) ||
        typeof info.name !== "string" || !info.name ||
        typeof info.fingerprint !== "string" || !info.fingerprint ||
        !["pending", "approved", "denied"].includes(info.status)
      ) {
        throw new TypeError("Invalid device authorization response");
      }
      this.update({ info });
      if (info.status === "approved" || info.status === "denied") {
        this.finish(info.status);
      } else {
        this.update({ phase: "pending" });
        this.tick();
      }
    } catch (reason) {
      if (operation !== this.operation) return;
      if (unavailableError(reason)) this.finish("unavailable");
      else this.update({ error: retryMessage(reason) });
    }
  };

  tick = (): void => {
    if (this.state.phase !== "pending" || !this.state.info) return;
    const remainingMs = Math.max(0, this.state.info.expires_at_ms - this.now());
    // An approval sent before the deadline can succeed after it. Let its
    // response win; a reauthentication retry still rechecks the deadline below.
    if (remainingMs === 0 && !this.state.busy) this.finish("expired");
    else this.update({ remainingMs });
  };

  approve = (): Promise<void> => this.decide("approved");
  deny = (): Promise<void> => this.decide("denied");

  private async decide(result: "approved" | "denied"): Promise<void> {
    this.tick();
    if (!this.request || this.state.phase !== "pending" || this.state.busy) {
      return;
    }
    const request = this.request;
    const operation = ++this.operation;
    this.update({ busy: true, error: null });
    const send = (): Promise<unknown> => {
      if (operation !== this.operation) {
        throw new DOMException("Cancelled", "AbortError");
      }
      if (!this.state.info || this.state.info.expires_at_ms <= this.now()) {
        throw new AuthApiError("Authorization expired", 410);
      }
      return result === "approved"
        ? this.dependencies.approve(request)
        : this.dependencies.deny(request);
    };
    try {
      if (result === "approved") await this.dependencies.authorize(send);
      else await send();
      if (operation === this.operation) this.finish(result);
    } catch (reason) {
      if (operation !== this.operation) return;
      if (unavailableError(reason)) this.finish("unavailable");
      else {
        this.update({ busy: false, error: retryMessage(reason) });
        this.tick();
      }
    }
  }
}
