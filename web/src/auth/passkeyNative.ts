import type { PasskeyOptions } from "./authApi";

type NativePasskeyRequest =
  | { action: "capabilities"; rp_id: string }
  | {
    action: "create" | "assert";
    rp_id: string;
    public_key: Record<string, unknown>;
  };

interface NativePasskeyGlobals {
  __cowboyNativePasskeyBridgeVersion?: number;
  __cowboyNativePasskey?: (request: NativePasskeyRequest) => Promise<unknown>;
}

interface NativePasskeyReply {
  ok?: unknown;
  available?: unknown;
  credential?: unknown;
  error?: {
    code?: unknown;
    message?: unknown;
  };
}

export class NativePasskeyBridgeError extends Error {
  constructor(
    message: string,
    readonly code: string,
  ) {
    super(message);
    this.name = "NativePasskeyBridgeError";
  }
}

function nativePasskeyGlobals(): NativePasskeyGlobals {
  return globalThis as typeof globalThis & NativePasskeyGlobals;
}

export function hasNativePasskeyBridge(): boolean {
  const root = nativePasskeyGlobals();
  return (root.__cowboyNativePasskeyBridgeVersion ?? 0) >= 1 &&
    typeof root.__cowboyNativePasskey === "function";
}

async function invokeNativePasskey(
  request: NativePasskeyRequest,
): Promise<NativePasskeyReply> {
  const bridge = nativePasskeyGlobals().__cowboyNativePasskey;
  if (!hasNativePasskeyBridge() || typeof bridge !== "function") {
    throw new NativePasskeyBridgeError(
      "The native Passkey bridge is unavailable.",
      "bridge_unavailable",
    );
  }
  let raw: unknown;
  try {
    raw = await bridge(request);
  } catch {
    throw new NativePasskeyBridgeError(
      "The native Passkey bridge could not be reached.",
      "bridge_unavailable",
    );
  }
  if (raw == null || typeof raw !== "object") {
    throw new NativePasskeyBridgeError(
      "The native Passkey bridge returned an invalid response.",
      "invalid_response",
    );
  }
  return raw as NativePasskeyReply;
}

export async function nativePasskeyAvailable(rpId: string): Promise<boolean> {
  if (!hasNativePasskeyBridge()) return false;
  try {
    const reply = await invokeNativePasskey({
      action: "capabilities",
      rp_id: rpId,
    });
    return reply.ok === true && reply.available === true;
  } catch {
    return false;
  }
}

function nativePasskeyError(reply: NativePasskeyReply): NativePasskeyBridgeError {
  const code = typeof reply.error?.code === "string"
    ? reply.error.code
    : "native_failure";
  const message = typeof reply.error?.message === "string" &&
      reply.error.message.trim() !== ""
    ? reply.error.message
    : "Native Passkey verification failed.";
  return new NativePasskeyBridgeError(message, code);
}

function validatedCredential(
  reply: NativePasskeyReply,
  action: "create" | "assert",
): Record<string, unknown> {
  if (reply.ok !== true) throw nativePasskeyError(reply);
  if (reply.credential == null || typeof reply.credential !== "object") {
    throw new NativePasskeyBridgeError(
      "The native Passkey bridge returned no credential.",
      "invalid_response",
    );
  }
  const credential = reply.credential as Record<string, unknown>;
  const response = credential.response;
  const requiredResponseFields = action === "create"
    ? ["clientDataJSON", "attestationObject"]
    : ["clientDataJSON", "authenticatorData", "signature"];
  if (
    typeof credential.id !== "string" ||
    typeof credential.rawId !== "string" ||
    credential.type !== "public-key" ||
    response == null || typeof response !== "object" ||
    requiredResponseFields.some((field) =>
      typeof (response as Record<string, unknown>)[field] !== "string"
    )
  ) {
    throw new NativePasskeyBridgeError(
      "The native Passkey bridge returned a malformed credential.",
      "invalid_response",
    );
  }
  return credential;
}

async function runNativePasskey(
  action: "create" | "assert",
  ceremony: PasskeyOptions,
): Promise<Record<string, unknown>> {
  const rpId = action === "create"
    ? String((ceremony.publicKey as Record<string, unknown>).rp &&
      ((ceremony.publicKey as Record<string, unknown>).rp as Record<string, unknown>).id || "")
    : String((ceremony.publicKey as Record<string, unknown>).rpId ?? "");
  if (rpId === "") {
    throw new NativePasskeyBridgeError(
      "Cowboy did not provide a Passkey relying-party ID.",
      "invalid_request",
    );
  }
  const reply = await invokeNativePasskey({
    action,
    rp_id: rpId,
    public_key: ceremony.publicKey as Record<string, unknown>,
  });
  return validatedCredential(reply, action);
}

export function createPasskeyNatively(
  ceremony: PasskeyOptions,
): Promise<Record<string, unknown>> {
  return runNativePasskey("create", ceremony);
}

export function assertPasskeyNatively(
  ceremony: PasskeyOptions,
): Promise<Record<string, unknown>> {
  return runNativePasskey("assert", ceremony);
}

export function nativePasskeyMayFallBack(reason: unknown): boolean {
  return reason instanceof NativePasskeyBridgeError &&
    new Set([
      "bridge_unavailable",
      "not_configured",
      "unsupported_os",
    ]).has(reason.code);
}
