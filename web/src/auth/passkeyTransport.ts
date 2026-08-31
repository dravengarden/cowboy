import { isNativeShell } from "../nativeShell";
import {
  hasNativeAuthenticationBrowser,
  hasNativeExternalOpener,
} from "../openExternal";
import { passkeysSupported } from "./passkeyBrowser";
import {
  hasNativePasskeyBridge,
  nativePasskeyAvailable,
} from "./passkeyNative";

export type PasskeyTransportKind = "native" | "browser" | "external";

export interface PasskeyTransportCapabilities {
  embeddedNativeShell: boolean;
  nativeBridgeAvailable: boolean;
  browserWebAuthn: boolean;
  externalAuthenticationBrowser: boolean;
}

/** Pure policy kept separate from runtime detection so every platform/fallback
 * combination has deterministic regression coverage. */
export function resolvePasskeyTransports(
  capabilities: PasskeyTransportCapabilities,
): PasskeyTransportKind[] {
  if (capabilities.nativeBridgeAvailable) {
    return capabilities.externalAuthenticationBrowser
      ? ["native", "external"]
      : ["native"];
  }
  if (capabilities.embeddedNativeShell) {
    return capabilities.externalAuthenticationBrowser ? ["external"] : [];
  }
  if (capabilities.browserWebAuthn) {
    return capabilities.externalAuthenticationBrowser
      ? ["browser", "external"]
      : ["browser"];
  }
  return capabilities.externalAuthenticationBrowser ? ["external"] : [];
}

function externalAuthenticationBrowserAvailable(): boolean {
  return hasNativeAuthenticationBrowser() || hasNativeExternalOpener();
}

export function passkeyTransportSupported(): boolean {
  return hasNativePasskeyBridge() || passkeysSupported() ||
    externalAuthenticationBrowserAvailable();
}

export function passkeyRegistrationNeedsUserGestureResume(): boolean {
  return hasNativePasskeyBridge() || !isNativeShell();
}

export async function currentPasskeyTransports(): Promise<
  PasskeyTransportKind[]
> {
  const nativeBridgeAvailable = hasNativePasskeyBridge() &&
    await nativePasskeyAvailable(globalThis.location.hostname);
  return resolvePasskeyTransports({
    embeddedNativeShell: isNativeShell(),
    nativeBridgeAvailable,
    browserWebAuthn: passkeysSupported(),
    externalAuthenticationBrowser: externalAuthenticationBrowserAvailable(),
  });
}

export function browserPasskeyMayFallBack(reason: unknown): boolean {
  return reason instanceof DOMException && reason.name === "SecurityError";
}
