import { isNativeShell } from "../nativeShell";
import {
  closeAuthenticationBrowser,
  hasNativeAuthenticationBrowser,
  openAuthenticationUrl,
} from "../openExternal";
import {
  authApi,
  AuthApiError,
  type ProductMe,
  type ProductOidcProvider,
} from "./authApi";
import { newPkceBinding } from "./pkce";

const POLL_INTERVAL_MS = 500;
const START_RACE_GRACE_MS = 10_000;
const AUTHORIZATION_TIMEOUT_MS = 5 * 60 * 1_000;

export function nativeOidcFlowSupported(): boolean {
  return isNativeShell() && hasNativeAuthenticationBrowser();
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

function retryablePollError(reason: unknown, startedAt: number): boolean {
  return reason instanceof TypeError ||
    reason instanceof AuthApiError &&
      (reason.status >= 500 ||
        reason.status === 401 && Date.now() - startedAt < START_RACE_GRACE_MS);
}

async function waitForDelay(signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
  await new Promise<void>((resolve, reject) => {
    const onAbort = (): void => {
      clearTimeout(timeout);
      reject(new DOMException("Cancelled", "AbortError"));
    };
    const timeout = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, POLL_INTERVAL_MS);
    signal?.addEventListener("abort", onAbort, { once: true });
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
  await waitForDelay(signal);
  while (Date.now() < deadline) {
    if (signal?.aborted) throw new DOMException("Cancelled", "AbortError");
    try {
      const result = await authApi.pollNativeOidc(
        provider,
        handoffToken,
        codeVerifier,
      );
      if ("status" in result) continue;
      return result;
    } catch (reason) {
      if (!retryablePollError(reason, startedAt)) throw reason;
    }
    await waitForDelay(signal);
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
  openAuthenticationUrl(
    nativeOidcStartUrl(
      location.origin,
      provider,
      codeBinding.challenge,
      handoffBinding.challenge,
    ),
  );
  try {
    return await waitForNativeOidc(
      provider,
      handoffBinding.verifier,
      codeBinding.verifier,
      signal,
    );
  } finally {
    closeAuthenticationBrowser();
  }
}
