import type { PasskeyOptions } from "./authApi";

export interface PasskeyDisplayContext {
  standalone: boolean;
  mobile: boolean;
}

/** webauthn-rs 0.5 emits `residentKey: "discouraged"` and no `hints`. Chrome
 * then ranks the hybrid QR sheet above Touch ID, and a Chromium desktop PWA
 * cannot see iCloud Keychain passkeys at all (crbug 364926914). Prefer the
 * local platform authenticator; keep hybrid as a desktop fallback. */
export const LOCAL_PASSKEY_HINTS = ["client-device", "hybrid"] as const;
export const MOBILE_PASSKEY_HINTS = ["client-device"] as const;

function bufferToBase64Url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll(
    "=",
    "",
  );
}

function base64UrlToBuffer(value: string): ArrayBuffer {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/");
  const binary = atob(
    padded.padEnd(padded.length + (4 - padded.length % 4) % 4, "="),
  );
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function mediaMatches(query: string): boolean {
  return globalThis.matchMedia?.(query).matches === true;
}

export function currentPasskeyDisplayContext(): PasskeyDisplayContext {
  return {
    standalone: mediaMatches("(display-mode: standalone)") ||
      mediaMatches("(display-mode: window-controls-overlay)") ||
      mediaMatches("(display-mode: minimal-ui)") ||
      (globalThis.navigator as Navigator & { standalone?: boolean })
          .standalone === true,
    mobile: mediaMatches("(pointer: coarse)") ||
      (globalThis.navigator?.maxTouchPoints ?? 0) > 0,
  };
}

function passkeyHints(display: PasskeyDisplayContext): string[] {
  return display.mobile
    ? [...MOBILE_PASSKEY_HINTS]
    : [...LOCAL_PASSKEY_HINTS];
}

function withLocalTransports(
  existing: unknown,
  display: PasskeyDisplayContext,
): AuthenticatorTransport[] {
  const transports = new Set<string>(
    Array.isArray(existing)
      ? existing.filter((item): item is string => typeof item === "string")
      : [],
  );
  transports.add("internal");
  if (display.mobile) transports.delete("hybrid");
  else transports.add("hybrid");
  return [...transports] as AuthenticatorTransport[];
}

export function shapePasskeyCreationPublicKey(
  options: Record<string, unknown>,
  display: PasskeyDisplayContext = currentPasskeyDisplayContext(),
): Record<string, unknown> {
  const selection = {
    ...((options.authenticatorSelection ?? {}) as Record<string, unknown>),
  };
  if (selection.residentKey !== "required") {
    selection.residentKey = "preferred";
    selection.requireResidentKey = false;
  }
  if (display.standalone) selection.authenticatorAttachment = "platform";
  return {
    ...options,
    hints: passkeyHints(display),
    authenticatorSelection: selection,
  };
}

export function shapePasskeyRequestPublicKey(
  options: Record<string, unknown>,
  display: PasskeyDisplayContext = currentPasskeyDisplayContext(),
): Record<string, unknown> {
  const allowCredentials = Array.isArray(options.allowCredentials)
    ? options.allowCredentials.map((item) => {
      const descriptor = { ...(item as Record<string, unknown>) };
      descriptor.transports = withLocalTransports(
        descriptor.transports,
        display,
      );
      return descriptor;
    })
    : options.allowCredentials;
  return {
    ...options,
    hints: passkeyHints(display),
    allowCredentials,
  };
}

function reviveCreateOptions(
  options: Record<string, unknown>,
): CredentialCreationOptions {
  const shaped = shapePasskeyCreationPublicKey(options);
  const publicKey = { ...shaped } as unknown as
    & PublicKeyCredentialCreationOptions
    & {
      challenge: BufferSource;
      user: PublicKeyCredentialUserEntity;
    };
  publicKey.challenge = base64UrlToBuffer(String(shaped.challenge));
  const user = { ...(shaped.user as PublicKeyCredentialUserEntity) };
  user.id = base64UrlToBuffer(String((shaped.user as { id: string }).id));
  publicKey.user = user;
  if (Array.isArray(shaped.excludeCredentials)) {
    publicKey.excludeCredentials = shaped.excludeCredentials.map((item) => {
      const descriptor = { ...(item as PublicKeyCredentialDescriptor) };
      descriptor.id = base64UrlToBuffer(String((item as { id: string }).id));
      return descriptor;
    });
  }
  return { publicKey };
}

function reviveRequestOptions(
  options: Record<string, unknown>,
): CredentialRequestOptions {
  const shaped = shapePasskeyRequestPublicKey(options);
  const publicKey = {
    ...shaped,
  } as unknown as PublicKeyCredentialRequestOptions;
  publicKey.challenge = base64UrlToBuffer(String(shaped.challenge));
  if (Array.isArray(shaped.allowCredentials)) {
    publicKey.allowCredentials = shaped.allowCredentials.map((item) => {
      const descriptor = { ...(item as PublicKeyCredentialDescriptor) };
      descriptor.id = base64UrlToBuffer(String((item as { id: string }).id));
      return descriptor;
    });
  }
  return { publicKey };
}

function credentialToJson(
  credential: PublicKeyCredential,
): Record<string, unknown> {
  const response = credential.response;
  const json: Record<string, unknown> = {
    id: credential.id,
    rawId: bufferToBase64Url(credential.rawId),
    type: credential.type,
    response: {},
    clientExtensionResults: credential.getClientExtensionResults(),
  };
  if (response instanceof AuthenticatorAttestationResponse) {
    json.response = {
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
      attestationObject: bufferToBase64Url(response.attestationObject),
    };
  } else if (response instanceof AuthenticatorAssertionResponse) {
    json.response = {
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
      authenticatorData: bufferToBase64Url(response.authenticatorData),
      signature: bufferToBase64Url(response.signature),
      userHandle: response.userHandle
        ? bufferToBase64Url(response.userHandle)
        : null,
    };
  }
  return json;
}

export function passkeysSupported(): boolean {
  return typeof globalThis.PublicKeyCredential === "function";
}

export async function createPasskey(
  ceremony: PasskeyOptions,
): Promise<Record<string, unknown>> {
  const credential = await navigator.credentials.create(
    reviveCreateOptions(ceremony.publicKey as Record<string, unknown>),
  );
  if (!(credential instanceof PublicKeyCredential)) {
    throw new Error("Passkey was not created");
  }
  return credentialToJson(credential);
}

export async function assertPasskey(
  ceremony: PasskeyOptions,
  signal?: AbortSignal,
): Promise<Record<string, unknown>> {
  const options = reviveRequestOptions(
    ceremony.publicKey as Record<string, unknown>,
  );
  if (signal !== undefined) options.signal = signal;
  const credential = await navigator.credentials.get(
    options,
  );
  if (!(credential instanceof PublicKeyCredential)) {
    throw new Error("Passkey was not asserted");
  }
  return credentialToJson(credential);
}
