import { assertEquals } from "jsr:@std/assert";
import { resolvePasskeyTransports } from "./passkeyTransport";

Deno.test("PWA uses the origin-bound browser WebAuthn ceremony", () => {
  assertEquals(resolvePasskeyTransports({
    embeddedNativeShell: false,
    nativeBridgeAvailable: false,
    browserWebAuthn: true,
    externalAuthenticationBrowser: false,
  }), ["browser"]);
});

Deno.test("SideStore shell uses native Passkeys only when signed capability exists", () => {
  assertEquals(resolvePasskeyTransports({
    embeddedNativeShell: true,
    nativeBridgeAvailable: true,
    browserWebAuthn: true,
    externalAuthenticationBrowser: true,
  }), ["native", "external"]);
  assertEquals(resolvePasskeyTransports({
    embeddedNativeShell: true,
    nativeBridgeAvailable: false,
    browserWebAuthn: true,
    externalAuthenticationBrowser: true,
  }), ["external"]);
});

Deno.test("macOS shell tries WebAuthn before its system-browser fallback", () => {
  assertEquals(resolvePasskeyTransports({
    embeddedNativeShell: false,
    nativeBridgeAvailable: false,
    browserWebAuthn: true,
    externalAuthenticationBrowser: true,
  }), ["browser", "external"]);
});

Deno.test("a broken native shell does not claim Passkey support", () => {
  assertEquals(resolvePasskeyTransports({
    embeddedNativeShell: true,
    nativeBridgeAvailable: false,
    browserWebAuthn: true,
    externalAuthenticationBrowser: false,
  }), []);
});
