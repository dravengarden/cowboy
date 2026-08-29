import { isNativeShell } from "../nativeShell";
import {
  closeAuthenticationBrowser,
  openAuthenticationUrl,
} from "../openExternal";
import {
  authApi,
  AuthApiError,
  type ExternalPasskeyAction,
  type ExternalPasskeyFinalize,
  type ProductMe,
  type ProductPasskey,
} from "./authApi";
import {
  assertPasskey as assertPasskeyInBrowser,
  createPasskey as createPasskeyInBrowser,
  passkeysSupported,
} from "./passkeyBrowser";

const POLL_INTERVAL_MS = 500;

function bytesToBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll(
    "=",
    "",
  );
}

async function newPkceBinding(): Promise<{
  verifier: string;
  challenge: string;
}> {
  const verifier = bytesToBase64Url(crypto.getRandomValues(new Uint8Array(32)));
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(verifier),
  );
  return {
    verifier,
    challenge: bytesToBase64Url(new Uint8Array(digest)),
  };
}

export function externalPasskeyUrl(
  origin: string,
  transactionId: string,
): string {
  const url = new URL("/passkey.html", origin);
  url.hash = new URLSearchParams({ transaction: transactionId }).toString();
  return url.href;
}

function retryablePollError(reason: unknown): boolean {
  return reason instanceof TypeError ||
    reason instanceof AuthApiError && reason.status >= 500;
}

async function waitForExternalPasskey(
  transactionId: string,
  verifier: string,
  expiresInSeconds: number,
): Promise<ExternalPasskeyFinalize> {
  const deadline = Date.now() + Math.max(1, expiresInSeconds) * 1_000;
  while (Date.now() < deadline) {
    try {
      const result = await authApi.finalizeExternalPasskey(
        transactionId,
        verifier,
      );
      if (result.status === "complete") return result;
    } catch (reason) {
      if (!retryablePollError(reason)) throw reason;
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
  }
  throw new AuthApiError("Passkey setup timed out. Please try again.", 408);
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
  openAuthenticationUrl(
    externalPasskeyUrl(location.origin, started.transaction_id),
  );
  try {
    return await waitForExternalPasskey(
      started.transaction_id,
      binding.verifier,
      started.expires_in_seconds,
    );
  } finally {
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
    if (reason.name === "NotAllowedError") {
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
