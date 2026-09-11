/** Cowboy-owned native port. No Plugin ID, Catalog lookup, native capability
 * name, registration API or arbitrary native dispatch crosses this boundary.
 * The installed v1 ceremony ABI remains unchanged during ownership migration.
 */
const PASSKEY_BRIDGE_VERSION = 1;
const MAX_BINARY_TEXT = 1024 * 1024;

type CreationOptions = Record<string, unknown> & {
  challenge: string;
  rp: Record<string, unknown> & { id: string };
  user: Record<string, unknown> & { id: string; name: string };
};
type AssertionOptions = Record<string, unknown> & {
  challenge: string;
  rpId: string;
};

type NativePasskeyRequest =
  | { action: "capabilities"; rp_id: string }
  | { action: "create"; rp_id: string; public_key: CreationOptions }
  | { action: "assert"; rp_id: string; public_key: AssertionOptions };

type NativePasskeyPort = (request: NativePasskeyRequest) => Promise<unknown>;
interface NativePasskeyGlobals {
  __cowboyNativePasskeyBridgeVersion?: unknown;
  __cowboyNativePasskey?: NativePasskeyPort;
}

type Credential<Response> = {
  id: string;
  rawId: string;
  type: "public-key";
  response: Response;
  clientExtensionResults: Record<string, never>;
};

export type CoreNativeRegistrationCredential = Credential<{
  clientDataJSON: string;
  attestationObject: string;
}>;
export type CoreNativeAssertionCredential = Credential<{
  clientDataJSON: string;
  authenticatorData: string;
  signature: string;
  userHandle: string | null;
}>;

const NATIVE_ERROR_MESSAGES = {
  not_configured: "This app signature is not configured for Cowboy Passkeys.",
  unsupported_os: "This OS version does not support native Passkeys.",
  cancelled: "Passkey verification was cancelled.",
  busy: "Another Passkey request is already active.",
  invalid_request: "Cowboy provided an invalid native Passkey request.",
  invalid_response: "The native Passkey bridge returned an invalid response.",
  native_failure: "Native Passkey verification failed.",
} as const;

export type NativePasskeyErrorCode =
  | keyof typeof NATIVE_ERROR_MESSAGES
  | "bridge_unavailable"
  | "outcome_unknown";

export class NativePasskeyBridgeError extends Error {
  constructor(message: string, readonly code: NativePasskeyErrorCode) {
    super(message);
    this.name = "NativePasskeyBridgeError";
  }
}

function fail(code: keyof typeof NATIVE_ERROR_MESSAGES): never {
  throw new NativePasskeyBridgeError(NATIVE_ERROR_MESSAGES[code], code);
}

function outcomeUnknown(): never {
  throw new NativePasskeyBridgeError(
    "The native Passkey result could not be confirmed. No automatic retry was started.",
    "outcome_unknown",
  );
}

function nativePasskeyPort(): NativePasskeyPort | undefined {
  const root = globalThis as typeof globalThis & NativePasskeyGlobals;
  return root.__cowboyNativePasskeyBridgeVersion === PASSKEY_BRIDGE_VERSION &&
      typeof root.__cowboyNativePasskey === "function"
    ? root.__cowboyNativePasskey
    : undefined;
}

export function hasNativePasskeyBridge(): boolean {
  return nativePasskeyPort() !== undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function exactFields(
  value: Record<string, unknown>,
  fields: readonly string[],
): boolean {
  const keys = Object.keys(value);
  return keys.length === fields.length &&
    keys.every((key) => fields.includes(key));
}

function binaryText(value: unknown): value is string {
  // Bounded wire decoding, not credential/signature verification. The
  // Controller remains the WebAuthn verifier and sole session authority.
  return typeof value === "string" && value.length > 0 &&
    value.length <= MAX_BINARY_TEXT && /^[A-Za-z0-9_-]+$/.test(value);
}

function relyingParty(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= 253 &&
    !/\s/u.test(value) &&
    [...value].every((char) =>
      char.charCodeAt(0) >= 32 && char.charCodeAt(0) !== 127
    );
}

async function invoke(
  port: NativePasskeyPort,
  request: NativePasskeyRequest,
): Promise<Record<string, unknown>> {
  let raw: unknown;
  try {
    // After dispatch, rejection is ambiguous: native may have created a
    // credential already. Never treat it as pre-dispatch unavailability.
    raw = await port(request);
  } catch {
    outcomeUnknown();
  }
  if (nativePasskeyPort() !== port) outcomeUnknown();
  if (!record(raw)) fail("invalid_response");
  if (raw.ok === false) {
    if (
      !exactFields(raw, ["ok", "error"]) || !record(raw.error) ||
      !exactFields(raw.error, ["code", "message"]) ||
      typeof raw.error.code !== "string" ||
      typeof raw.error.message !== "string" ||
      raw.error.message.length > 4096 ||
      !Object.hasOwn(NATIVE_ERROR_MESSAGES, raw.error.code)
    ) fail("invalid_response");
    // Error text is core-owned too; do not echo arbitrary native payloads.
    fail(raw.error.code as keyof typeof NATIVE_ERROR_MESSAGES);
  }
  if (raw.ok !== true) fail("invalid_response");
  return raw;
}

export async function nativePasskeyAvailable(rpId: string): Promise<boolean> {
  const port = nativePasskeyPort();
  if (!port || !relyingParty(rpId)) return false;
  try {
    const reply = await invoke(port, { action: "capabilities", rp_id: rpId });
    return exactFields(reply, ["ok", "available"]) && reply.available === true;
  } catch {
    // Capabilities have no ceremony effect and may fail unavailable.
    return false;
  }
}

function credential(reply: Record<string, unknown>): {
  id: string;
  response: Record<string, unknown>;
} {
  const value = reply.credential;
  if (
    !exactFields(reply, ["ok", "credential"]) || !record(value) ||
    !exactFields(value, [
      "id",
      "rawId",
      "type",
      "response",
      "clientExtensionResults",
    ]) ||
    !binaryText(value.id) || value.id !== value.rawId ||
    value.type !== "public-key" ||
    !record(value.response) || !record(value.clientExtensionResults) ||
    !exactFields(value.clientExtensionResults, [])
  ) fail("invalid_response");
  return { id: value.id, response: value.response };
}

let ceremonyActive = false;

async function runCeremony<T>(
  request: Exclude<NativePasskeyRequest, { action: "capabilities" }>,
  decode: (reply: Record<string, unknown>) => T,
): Promise<T> {
  const port = nativePasskeyPort();
  if (!port) {
    throw new NativePasskeyBridgeError(
      "The native Passkey bridge is unavailable.",
      "bridge_unavailable",
    );
  }
  if (ceremonyActive) fail("busy");
  ceremonyActive = true;
  try {
    return decode(await invoke(port, request));
  } finally {
    ceremonyActive = false;
  }
}

export async function createCoreNativePasskey(
  options: Record<string, unknown>,
): Promise<CoreNativeRegistrationCredential> {
  if (
    !record(options) || !binaryText(options.challenge) || !record(options.rp) ||
    !relyingParty(options.rp.id) || !record(options.user) ||
    !binaryText(options.user.id) || typeof options.user.name !== "string" ||
    options.user.name.trim() === "" || options.user.name.length > 4096
  ) fail("invalid_request");
  return await runCeremony({
    action: "create",
    rp_id: options.rp.id,
    public_key: {
      ...options,
      challenge: options.challenge,
      rp: { ...options.rp, id: options.rp.id },
      user: { ...options.user, id: options.user.id, name: options.user.name },
    },
  }, (reply) => {
    const { id, response } = credential(reply);
    if (
      !exactFields(response, ["clientDataJSON", "attestationObject"]) ||
      !binaryText(response.clientDataJSON) ||
      !binaryText(response.attestationObject)
    ) fail("invalid_response");
    return {
      id,
      rawId: id,
      type: "public-key",
      response: {
        clientDataJSON: response.clientDataJSON,
        attestationObject: response.attestationObject,
      },
      clientExtensionResults: {},
    };
  });
}

export async function assertCoreNativePasskey(
  options: Record<string, unknown>,
): Promise<CoreNativeAssertionCredential> {
  if (
    !record(options) || !binaryText(options.challenge) ||
    !relyingParty(options.rpId)
  ) {
    fail("invalid_request");
  }
  return await runCeremony({
    action: "assert",
    rp_id: options.rpId,
    public_key: {
      ...options,
      challenge: options.challenge,
      rpId: options.rpId,
    },
  }, (reply) => {
    const { id, response } = credential(reply);
    if (
      !exactFields(response, [
        "clientDataJSON",
        "authenticatorData",
        "signature",
        "userHandle",
      ]) ||
      !binaryText(response.clientDataJSON) ||
      !binaryText(response.authenticatorData) ||
      !binaryText(response.signature) ||
      (response.userHandle !== null && !binaryText(response.userHandle))
    ) fail("invalid_response");
    return {
      id,
      rawId: id,
      type: "public-key",
      response: {
        clientDataJSON: response.clientDataJSON,
        authenticatorData: response.authenticatorData,
        signature: response.signature,
        userHandle: response.userHandle,
      },
      clientExtensionResults: {},
    };
  });
}

export function nativePasskeyMayFallBack(reason: unknown): boolean {
  return reason instanceof NativePasskeyBridgeError &&
    (reason.code === "bridge_unavailable" || reason.code === "not_configured" ||
      reason.code === "unsupported_os");
}
