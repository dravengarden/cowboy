import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  createPasskeyNatively,
  hasNativePasskeyBridge,
  nativePasskeyAvailable,
  NativePasskeyBridgeError,
  nativePasskeyMayFallBack,
} from "./passkeyNative";

interface TestNativeGlobals {
  __cowboyNativePasskeyBridgeVersion?: number;
  __cowboyNativePasskey?: (request: unknown) => Promise<unknown>;
}

const root = globalThis as typeof globalThis & TestNativeGlobals;

function clearBridge(): void {
  delete root.__cowboyNativePasskeyBridgeVersion;
  delete root.__cowboyNativePasskey;
}

Deno.test("native Passkey capability is explicit and RP scoped", async () => {
  clearBridge();
  assertEquals(hasNativePasskeyBridge(), false);
  root.__cowboyNativePasskeyBridgeVersion = 1;
  root.__cowboyNativePasskey = async (request) => ({
    ok: true,
    available: (request as { rp_id?: string }).rp_id === "cowboy.example",
  });
  try {
    assertEquals(hasNativePasskeyBridge(), true);
    assertEquals(await nativePasskeyAvailable("cowboy.example"), true);
    assertEquals(await nativePasskeyAvailable("other.example"), false);
  } finally {
    clearBridge();
  }
});

Deno.test("native registration emits standard WebAuthn JSON", async () => {
  root.__cowboyNativePasskeyBridgeVersion = 1;
  root.__cowboyNativePasskey = async () => ({
    ok: true,
    credential: {
      id: "credential",
      rawId: "credential",
      type: "public-key",
      response: {
        clientDataJSON: "client-data",
        attestationObject: "attestation",
      },
      clientExtensionResults: {},
    },
  });
  try {
    const result = await createPasskeyNatively({
      publicKey: {
        challenge: "challenge",
        rp: { id: "cowboy.example", name: "Cowboy" },
        user: { id: "user", name: "owner", displayName: "Owner" },
      },
    });
    assertEquals(result.type, "public-key");
    assertEquals(
      (result.response as Record<string, unknown>).attestationObject,
      "attestation",
    );
  } finally {
    clearBridge();
  }
});

Deno.test("only unavailable native capability may fall back", async () => {
  root.__cowboyNativePasskeyBridgeVersion = 1;
  root.__cowboyNativePasskey = async () => ({
    ok: false,
    error: { code: "cancelled", message: "Cancelled" },
  });
  try {
    const error = await assertRejects(
      () => createPasskeyNatively({
        publicKey: {
          challenge: "challenge",
          rp: { id: "cowboy.example" },
          user: { id: "user", name: "owner" },
        },
      }),
      NativePasskeyBridgeError,
    );
    assertEquals(nativePasskeyMayFallBack(error), false);
    assertEquals(
      nativePasskeyMayFallBack(
        new NativePasskeyBridgeError("Not configured", "not_configured"),
      ),
      true,
    );
  } finally {
    clearBridge();
  }
});
