import type { PasskeyOptions } from "./authApi";
import {
  assertCoreNativePasskey,
  type CoreNativeAssertionCredential,
  type CoreNativeRegistrationCredential,
  createCoreNativePasskey,
} from "../coreNativeBridge";

export {
  hasNativePasskeyBridge,
  nativePasskeyAvailable,
  NativePasskeyBridgeError,
  nativePasskeyMayFallBack,
} from "../coreNativeBridge";

export function createPasskeyNatively(
  ceremony: PasskeyOptions,
): Promise<CoreNativeRegistrationCredential> {
  return createCoreNativePasskey(ceremony.publicKey);
}

export function assertPasskeyNatively(
  ceremony: PasskeyOptions,
): Promise<CoreNativeAssertionCredential> {
  return assertCoreNativePasskey(ceremony.publicKey);
}
