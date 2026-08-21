import type { PasskeyCeremony } from "./authApi";

function bufferToBase64Url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function base64UrlToBuffer(value: string): ArrayBuffer {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/");
  const binary = atob(padded.padEnd(padded.length + (4 - padded.length % 4) % 4, "="));
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function reviveCreateOptions(
  options: Record<string, unknown>,
): CredentialCreationOptions {
  const publicKey = { ...options } as unknown as PublicKeyCredentialCreationOptions & {
    challenge: BufferSource;
    user: PublicKeyCredentialUserEntity;
  };
  publicKey.challenge = base64UrlToBuffer(String(options.challenge));
  const user = { ...(options.user as PublicKeyCredentialUserEntity) };
  user.id = base64UrlToBuffer(String((options.user as { id: string }).id));
  publicKey.user = user;
  if (Array.isArray(options.excludeCredentials)) {
    publicKey.excludeCredentials = options.excludeCredentials.map((item) => {
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
  const publicKey = { ...options } as unknown as PublicKeyCredentialRequestOptions;
  publicKey.challenge = base64UrlToBuffer(String(options.challenge));
  if (Array.isArray(options.allowCredentials)) {
    publicKey.allowCredentials = options.allowCredentials.map((item) => {
      const descriptor = { ...(item as PublicKeyCredentialDescriptor) };
      descriptor.id = base64UrlToBuffer(String((item as { id: string }).id));
      return descriptor;
    });
  }
  return { publicKey };
}

function credentialToJson(credential: PublicKeyCredential): Record<string, unknown> {
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
  ceremony: PasskeyCeremony,
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
  ceremony: PasskeyCeremony,
): Promise<Record<string, unknown>> {
  const credential = await navigator.credentials.get(
    reviveRequestOptions(ceremony.publicKey as Record<string, unknown>),
  );
  if (!(credential instanceof PublicKeyCredential)) {
    throw new Error("Passkey was not asserted");
  }
  return credentialToJson(credential);
}
