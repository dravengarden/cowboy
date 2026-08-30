import { isNativeShell } from "../nativeShell";
import {
  closeAuthenticationBrowser,
  NATIVE_APP_RESUMED_EVENT,
  NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT,
  openAuthenticationUrl,
} from "../openExternal";
import {
  authApi,
  AuthApiError,
  type ExternalPasskeyAction,
  externalPasskeyApi,
  type ExternalPasskeyFinalize,
  type ProductMe,
  type ProductPasskey,
} from "./authApi";
import {
  assertPasskey as assertPasskeyInBrowser,
  createPasskey as createPasskeyInBrowser,
  passkeysSupported,
} from "./passkeyBrowser";
import { newPkceBinding } from "./pkce";

const RECONNECT_INITIAL_MS = 250;
const RECONNECT_MAX_MS = 4_000;
const NATIVE_CLOSE_RECONCILE_ATTEMPTS = 10;
const NATIVE_CLOSE_RECONCILE_DELAY_MS = 250;

type ExternalPasskeyEventStatus =
  | "complete"
  | "failed"
  | "unavailable"
  | "browser-closed"
  | "initiator-resumed";

interface ExternalPasskeyEventDependencies {
  createSocket: (url: string) => WebSocket;
  origin: string;
}

interface ExternalPasskeyCloseReconcileDependencies {
  finalize: (
    transactionId: string,
    verifier: string,
  ) => Promise<ExternalPasskeyFinalize>;
  fail: (transactionId: string) => Promise<unknown>;
  wait: (delayMs: number, signal: AbortSignal) => Promise<void>;
}

export function externalPasskeyUrl(
  origin: string,
  transactionId: string,
): string {
  const url = new URL("/passkey.html", origin);
  url.hash = new URLSearchParams({ transaction: transactionId }).toString();
  return url.href;
}

export function externalPasskeyEventsUrl(origin: string): string {
  const url = new URL(
    "/api/auth/passkeys/external/events",
    new URL(origin).origin,
  );
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.href;
}

function retryableEventError(reason: unknown): boolean {
  return reason instanceof TypeError ||
    reason instanceof AuthApiError && reason.status >= 500;
}

function passkeyTimeoutError(): AuthApiError {
  return new AuthApiError("Passkey setup timed out. Please try again.", 408);
}

function passkeyAbortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("Cancelled", "AbortError");
}

async function waitForDelay(
  delayMs: number,
  signal: AbortSignal,
): Promise<void> {
  if (signal.aborted) throw passkeyAbortReason(signal);
  await new Promise<void>((resolve, reject) => {
    const onAbort = (): void => {
      clearTimeout(timer);
      reject(passkeyAbortReason(signal));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, delayMs);
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

export async function waitForExternalPasskeyEvent(
  transactionId: string,
  verifier: string,
  signal: AbortSignal,
  timeoutMs: number,
  dependencies: Partial<ExternalPasskeyEventDependencies> = {},
): Promise<ExternalPasskeyEventStatus> {
  if (signal.aborted) throw passkeyAbortReason(signal);
  return await new Promise<ExternalPasskeyEventStatus>((resolve, reject) => {
    const origin = dependencies.origin ?? location.origin;
    const socket = (dependencies.createSocket ?? ((url) => new WebSocket(url)))(
      externalPasskeyEventsUrl(origin),
    );
    let settled = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const finish = (
      result: { status: ExternalPasskeyEventStatus } | { error: unknown },
    ): void => {
      if (settled) return;
      settled = true;
      if (timeout !== undefined) clearTimeout(timeout);
      signal.removeEventListener("abort", onAbort);
      globalThis.removeEventListener(
        NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT,
        onNativeBrowserClosed,
      );
      globalThis.removeEventListener(
        NATIVE_APP_RESUMED_EVENT,
        onNativeAppResumed,
      );
      socket.close();
      if ("status" in result) resolve(result.status);
      else reject(result.error);
    };
    const onAbort = (): void =>
      finish({ error: passkeyAbortReason(signal) });
    // SFSafariViewController backgrounds the WKWebView that owns this socket.
    // Closing a successfully completed ceremony must wake the initiator so it
    // can perform the PKCE-bound durable finalize, not be treated as cancel.
    const onNativeBrowserClosed = (): void =>
      finish({ status: "browser-closed" });
    // The native dismissal callback can run while WKWebView is suspended and
    // lose its JavaScript event. UIApplication.didBecomeActive is repaired by
    // the shell before it emits this second, idempotent wake-up signal.
    const onNativeAppResumed = (): void =>
      finish({ status: "initiator-resumed" });
    signal.addEventListener("abort", onAbort, { once: true });
    globalThis.addEventListener(
      NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT,
      onNativeBrowserClosed,
      { once: true },
    );
    globalThis.addEventListener(
      NATIVE_APP_RESUMED_EVENT,
      onNativeAppResumed,
      { once: true },
    );
    timeout = setTimeout(
      () =>
        finish({
          error: passkeyTimeoutError(),
        }),
      Math.max(1, timeoutMs),
    );
    socket.addEventListener("open", () => {
      socket.send(JSON.stringify({
        transaction_id: transactionId,
        code_verifier: verifier,
      }));
    }, { once: true });
    socket.addEventListener("message", (event) => {
      try {
        const parsed = JSON.parse(String(event.data)) as { status?: unknown };
        if (
          parsed.status === "complete" || parsed.status === "failed" ||
          parsed.status === "unavailable"
        ) {
          finish({ status: parsed.status });
          return;
        }
      } catch {
        // A malformed public event is a transport failure, never completion.
      }
      finish({ error: new TypeError("Invalid Passkey event") });
    });
    socket.addEventListener("error", () => {
      finish({ error: new TypeError("Passkey event socket failed") });
    }, { once: true });
    socket.addEventListener("close", () => {
      finish({ error: new TypeError("Passkey event socket closed") });
    }, { once: true });
  });
}

export async function reconcileExternalPasskeyAfterResume(
  transactionId: string,
  verifier: string,
  signal: AbortSignal,
  dependencies: Partial<ExternalPasskeyCloseReconcileDependencies> = {},
): Promise<ExternalPasskeyFinalize> {
  const finalize = dependencies.finalize ??
    ((id, codeVerifier) =>
      authApi.finalizeExternalPasskey(id, codeVerifier, signal));
  const wait = dependencies.wait ?? waitForDelay;
  let sawPending = false;
  let lastRetryableError: unknown;

  for (let attempt = 0; attempt < NATIVE_CLOSE_RECONCILE_ATTEMPTS; attempt++) {
    if (signal.aborted) throw passkeyAbortReason(signal);
    try {
      const result = await finalize(transactionId, verifier);
      if (result.status === "complete") return result;
      sawPending = true;
      lastRetryableError = undefined;
    } catch (reason) {
      if (!retryableEventError(reason)) throw reason;
      lastRetryableError = reason;
    }
    if (attempt + 1 < NATIVE_CLOSE_RECONCILE_ATTEMPTS) {
      await wait(NATIVE_CLOSE_RECONCILE_DELAY_MS, signal);
    }
  }

  if (!sawPending && lastRetryableError) throw lastRetryableError;
  return { status: "pending" };
}

export async function reconcileExternalPasskeyAfterBrowserClose(
  transactionId: string,
  verifier: string,
  signal: AbortSignal,
  dependencies: Partial<ExternalPasskeyCloseReconcileDependencies> = {},
): Promise<ExternalPasskeyFinalize> {
  const finalize = dependencies.finalize ??
    ((id, codeVerifier) =>
      authApi.finalizeExternalPasskey(id, codeVerifier, signal));
  const fail = dependencies.fail ?? externalPasskeyApi.fail;
  const wait = dependencies.wait ?? waitForDelay;
  let lastRetryableError: unknown;

  for (let attempt = 0; attempt < NATIVE_CLOSE_RECONCILE_ATTEMPTS; attempt++) {
    if (signal.aborted) throw passkeyAbortReason(signal);
    try {
      const result = await finalize(transactionId, verifier);
      if (result.status === "complete") return result;
      lastRetryableError = undefined;
    } catch (reason) {
      if (!retryableEventError(reason)) throw reason;
      lastRetryableError = reason;
    }
    if (attempt + 1 < NATIVE_CLOSE_RECONCILE_ATTEMPTS) {
      await wait(NATIVE_CLOSE_RECONCILE_DELAY_MS, signal);
    }
  }

  await fail(transactionId).catch(() => undefined);
  if (lastRetryableError) {
    throw new AuthApiError(
      "Cowboy could not confirm the completed Passkey. Please try again.",
      503,
    );
  }
  throw new DOMException("Cancelled", "AbortError");
}

async function waitForExternalPasskey(
  transactionId: string,
  verifier: string,
  expiresInSeconds: number,
  signal: AbortSignal,
): Promise<ExternalPasskeyFinalize> {
  const deadline = Date.now() + Math.max(1, expiresInSeconds) * 1_000;
  let reconnectDelay = RECONNECT_INITIAL_MS;
  while (Date.now() < deadline) {
    if (signal.aborted) throw passkeyAbortReason(signal);
    try {
      const remainingMs = Math.max(1, deadline - Date.now());
      const status = await waitForExternalPasskeyEvent(
        transactionId,
        verifier,
        signal,
        remainingMs,
      );
      if (status === "failed") {
        throw new DOMException("Cancelled", "AbortError");
      }
      if (status === "unavailable") {
        throw new AuthApiError("Passkey setup is no longer active.", 410);
      }
      if (status === "browser-closed") {
        return await reconcileExternalPasskeyAfterBrowserClose(
          transactionId,
          verifier,
          signal,
        );
      }
      if (status === "initiator-resumed") {
        const result = await reconcileExternalPasskeyAfterResume(
          transactionId,
          verifier,
          signal,
        );
        if (result.status === "complete") return result;
        reconnectDelay = RECONNECT_INITIAL_MS;
        continue;
      }
      if (status === "complete") {
        const result = await authApi.finalizeExternalPasskey(
          transactionId,
          verifier,
          signal,
        );
        if (result.status === "complete") return result;
      }
      reconnectDelay = RECONNECT_INITIAL_MS;
    } catch (reason) {
      if (signal.aborted) throw passkeyAbortReason(signal);
      if (!retryableEventError(reason)) throw reason;
    }
    await waitForDelay(reconnectDelay, signal);
    reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
  }
  throw passkeyTimeoutError();
}

async function runExternalPasskey(
  action: ExternalPasskeyAction,
  nickname?: string,
): Promise<ExternalPasskeyFinalize> {
  const binding = await newPkceBinding();
  const started = await authApi.startExternalPasskey(
    action,
    binding.challenge,
    nickname,
  );
  const flow = new AbortController();
  const timeout = setTimeout(
    () => flow.abort(passkeyTimeoutError()),
    Math.max(1, started.expires_in_seconds) * 1_000,
  );
  try {
    openAuthenticationUrl(
      externalPasskeyUrl(location.origin, started.transaction_id),
    );
    return await waitForExternalPasskey(
      started.transaction_id,
      binding.verifier,
      started.expires_in_seconds,
      flow.signal,
    );
  } catch (reason) {
    await externalPasskeyApi.fail(started.transaction_id).catch(() =>
      undefined
    );
    throw reason;
  } finally {
    clearTimeout(timeout);
    closeAuthenticationBrowser();
  }
}

export function passkeyFlowSupported(): boolean {
  return isNativeShell() || passkeysSupported();
}

export async function registerPasskey(
  nickname: string,
): Promise<ProductPasskey> {
  if (isNativeShell()) {
    const result = await runExternalPasskey("register", nickname);
    if (result.status === "complete" && "passkey" in result) {
      return result.passkey;
    }
    throw new Error("Passkey registration returned an invalid response");
  }
  const ceremony = await authApi.startPasskeyRegister(nickname);
  const credential = await createPasskeyInBrowser(ceremony);
  return await authApi.completePasskeyRegister(
    ceremony.challenge_id,
    credential,
  );
}

export async function verifyPasskey(): Promise<ProductMe> {
  if (isNativeShell()) {
    const result = await runExternalPasskey("assert");
    if (result.status === "complete" && "me" in result) return result.me;
    throw new Error("Passkey verification returned an invalid response");
  }
  const ceremony = await authApi.startPasskeyAssert();
  const credential = await assertPasskeyInBrowser(ceremony);
  return await authApi.completePasskeyAssert(
    ceremony.challenge_id,
    credential,
  );
}

export function passkeyErrorMessage(
  reason: unknown,
  fallback: string,
): string {
  if (reason instanceof AuthApiError) return reason.message;
  if (reason instanceof DOMException) {
    if (reason.name === "AbortError" || reason.name === "NotAllowedError") {
      return "Passkey verification was cancelled or timed out.";
    }
    if (reason.name === "InvalidStateError") {
      return "This Passkey is already registered.";
    }
    if (reason.name === "SecurityError") {
      return "This browser cannot use Passkeys for this Cowboy address.";
    }
  }
  return fallback;
}

export function passkeyFlowCancelled(reason: unknown): boolean {
  if (reason instanceof DOMException) {
    return reason.name === "AbortError" || reason.name === "NotAllowedError";
  }
  return reason instanceof AuthApiError &&
    reason.message === "Passkey setup was cancelled";
}
