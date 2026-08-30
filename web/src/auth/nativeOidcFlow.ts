import { isNativeShell } from "../nativeShell";
import {
  closeAuthenticationBrowser,
  hasNativeAuthenticationBrowser,
  NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT,
  openAuthenticationUrlConfirmed,
} from "../openExternal";
import {
  authApi,
  AuthApiError,
  nativeOidcEventsPath,
  type ProductMe,
  type ProductOidcProvider,
} from "./authApi";
import { newPkceBinding } from "./pkce";

const RECONNECT_INITIAL_MS = 250;
const RECONNECT_MAX_MS = 4_000;
const START_RACE_GRACE_MS = 10_000;
const AUTHORIZATION_TIMEOUT_MS = 5 * 60 * 1_000;

type NativeOidcEventStatus = "ready" | "failed" | "unavailable";

export function nativeOidcFlowSupported(): boolean {
  return isNativeShell() && hasNativeAuthenticationBrowser();
}

export function browserOidcFlowSupported(): boolean {
  return typeof window !== "undefined" && typeof window.open === "function";
}

export function nativeOidcStartUrl(
  origin: string,
  provider: ProductOidcProvider,
  codeChallenge: string,
  handoffChallenge: string,
): string {
  const expectedOrigin = new URL(origin).origin;
  const url = new URL(provider.start_url, expectedOrigin);
  if (url.origin !== expectedOrigin) {
    throw new Error("External sign-in must start on this Cowboy server");
  }
  url.search = new URLSearchParams({
    client: "browser-shell",
    code_challenge: codeChallenge,
    handoff_challenge: handoffChallenge,
  }).toString();
  return url.href;
}

export function nativeOidcEventsUrl(
  origin: string,
  provider: ProductOidcProvider,
): string {
  const url = new URL(nativeOidcEventsPath(provider), new URL(origin).origin);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.href;
}

function retryablePollError(reason: unknown, startedAt: number): boolean {
  return reason instanceof TypeError ||
    reason instanceof AuthApiError &&
      (reason.status >= 500 ||
        reason.status === 401 && Date.now() - startedAt < START_RACE_GRACE_MS);
}

async function waitForDelay(
  delayMs: number,
  signal?: AbortSignal,
): Promise<void> {
  if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
  await new Promise<void>((resolve, reject) => {
    const onAbort = (): void => {
      clearTimeout(timeout);
      reject(new DOMException("Cancelled", "AbortError"));
    };
    const timeout = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, delayMs);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

async function waitForNativeOidcEvent(
  provider: ProductOidcProvider,
  handoffToken: string,
  codeVerifier: string,
  signal?: AbortSignal,
): Promise<NativeOidcEventStatus> {
  if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
  return await new Promise<NativeOidcEventStatus>((resolve, reject) => {
    const socket = new WebSocket(
      nativeOidcEventsUrl(globalThis.location.origin, provider),
    );
    let settled = false;
    const finish = (
      result: { status: NativeOidcEventStatus } | { error: unknown },
    ): void => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener("abort", onAbort);
      socket.close();
      if ("status" in result) resolve(result.status);
      else reject(result.error);
    };
    const onAbort = (): void =>
      finish({ error: new DOMException("Cancelled", "AbortError") });
    signal?.addEventListener("abort", onAbort, { once: true });
    socket.addEventListener("open", () => {
      socket.send(JSON.stringify({
        handoff_token: handoffToken,
        code_verifier: codeVerifier,
      }));
    }, { once: true });
    socket.addEventListener("message", (event) => {
      try {
        const parsed = JSON.parse(String(event.data)) as { status?: unknown };
        if (
          parsed.status === "ready" || parsed.status === "failed" ||
          parsed.status === "unavailable"
        ) {
          finish({ status: parsed.status });
          return;
        }
      } catch {
        // A malformed public handoff event is a transport failure, never auth.
      }
      finish({ error: new TypeError("Invalid native authorization event") });
    });
    socket.addEventListener("error", () => {
      finish({ error: new TypeError("Native authorization socket failed") });
    }, { once: true });
    socket.addEventListener("close", () => {
      finish({ error: new TypeError("Native authorization socket closed") });
    }, { once: true });
  });
}

async function waitForNativeOidc(
  provider: ProductOidcProvider,
  handoffToken: string,
  codeVerifier: string,
  signal?: AbortSignal,
): Promise<ProductMe> {
  const startedAt = Date.now();
  const deadline = startedAt + AUTHORIZATION_TIMEOUT_MS;
  let reconnectDelay = RECONNECT_INITIAL_MS;
  while (Date.now() < deadline) {
    if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
    try {
      const status = await waitForNativeOidcEvent(
        provider,
        handoffToken,
        codeVerifier,
        signal,
      );
      if (status === "failed") {
        throw new AuthApiError("External authorization was denied.", 410);
      }
      if (
        status === "unavailable" &&
        Date.now() - startedAt >= START_RACE_GRACE_MS
      ) {
        throw new AuthApiError("External authorization is no longer active.", 401);
      }
      if (status === "ready") {
        const result = await authApi.pollNativeOidc(
          provider,
          handoffToken,
          codeVerifier,
        );
        if (!("status" in result)) return result;
      }
      reconnectDelay = RECONNECT_INITIAL_MS;
    } catch (reason) {
      if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
      if (!retryablePollError(reason, startedAt)) throw reason;
    }
    await waitForDelay(reconnectDelay, signal);
    reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
  }
  throw new AuthApiError("External sign-in timed out. Please try again.", 408);
}

export async function runNativeOidc(
  provider: ProductOidcProvider,
  signal?: AbortSignal,
): Promise<ProductMe> {
  if (!nativeOidcFlowSupported()) {
    throw new Error("Native authentication browser is unavailable");
  }
  const [codeBinding, handoffBinding] = await Promise.all([
    newPkceBinding(),
    newPkceBinding(),
  ]);
  const flow = new AbortController();
  const cancel = (): void => flow.abort();
  if (signal?.aborted) cancel();
  signal?.addEventListener("abort", cancel, { once: true });
  globalThis.addEventListener(
    NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT,
    cancel,
  );
  try {
    if (flow.signal.aborted) throw new DOMException("Cancelled", "AbortError");
    await openAuthenticationUrlConfirmed(
      nativeOidcStartUrl(
        location.origin,
        provider,
        codeBinding.challenge,
        handoffBinding.challenge,
      ),
      flow.signal,
    );
    return await waitForNativeOidc(
      provider,
      handoffBinding.verifier,
      codeBinding.verifier,
      flow.signal,
    );
  } catch (reason) {
    void authApi.cancelNativeOidc(
      provider,
      handoffBinding.verifier,
      codeBinding.verifier,
    ).catch(() => undefined);
    throw reason;
  } finally {
    signal?.removeEventListener("abort", cancel);
    globalThis.removeEventListener(
      NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT,
      cancel,
    );
    closeAuthenticationBrowser();
  }
}

/**
 * Keep browser/PWA state alive while an external Provider verifies the user.
 * The blank window is opened synchronously from the click before PKCE work so
 * iOS does not block it. Its opener is severed before any Provider navigation;
 * Cowboy retains only the capability to close the window after the PKCE-bound
 * WebSocket handoff completes.
 */
export async function runBrowserOidc(
  provider: ProductOidcProvider,
  signal?: AbortSignal,
): Promise<ProductMe> {
  if (!browserOidcFlowSupported()) {
    throw new Error("Browser authentication window is unavailable");
  }
  const popup = window.open("about:blank", "_blank");
  if (!popup) {
    throw new AuthApiError(
      "Cowboy could not open the secure sign-in window. Allow pop-ups and try again.",
      400,
    );
  }
  try {
    popup.opener = null;
    if (popup.opener !== null) {
      throw new Error("Cowboy could not isolate the secure sign-in window");
    }
  } catch (reason) {
    popup.close();
    throw reason;
  }

  const flow = new AbortController();
  const cancel = (): void => flow.abort();
  if (signal?.aborted) cancel();
  signal?.addEventListener("abort", cancel, { once: true });
  let codeBinding: Awaited<ReturnType<typeof newPkceBinding>> | undefined;
  let handoffBinding: Awaited<ReturnType<typeof newPkceBinding>> | undefined;
  try {
    [codeBinding, handoffBinding] = await Promise.all([
      newPkceBinding(),
      newPkceBinding(),
    ]);
    if (flow.signal.aborted) {
      throw new DOMException("Cancelled", "AbortError");
    }
    if (popup.closed) {
      throw new DOMException("Cancelled", "AbortError");
    }
    popup.location.replace(
      nativeOidcStartUrl(
        location.origin,
        provider,
        codeBinding.challenge,
        handoffBinding.challenge,
      ),
    );
    return await waitForNativeOidc(
      provider,
      handoffBinding.verifier,
      codeBinding.verifier,
      flow.signal,
    );
  } catch (reason) {
    if (codeBinding && handoffBinding) {
      void authApi.cancelNativeOidc(
        provider,
        handoffBinding.verifier,
        codeBinding.verifier,
      ).catch(() => undefined);
    }
    throw reason;
  } finally {
    signal?.removeEventListener("abort", cancel);
    popup.close();
  }
}
