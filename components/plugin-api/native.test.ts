import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  type CowboyNativePluginHost,
  installPluginRuntimeHosts,
  invokeNativePluginCapability,
  PLUGIN_NATIVE_HOST_API_VERSION,
  supportsNativePluginCapability,
} from "./types.ts";

const descriptor = {
  id: "passkey",
  generation: "a".repeat(64),
  slots: ["account.panel"],
  ui: {
    schema_version: 1,
    renderers: { "account.panel": "account-passkeys-v1" },
  },
  native_capabilities: ["webauthn"],
};

Deno.test("native plugin ABI intersects signed and shell capabilities", async () => {
  const root = globalThis as typeof globalThis & {
    __COWBOY_NATIVE_PLUGIN_HOST?: CowboyNativePluginHost;
  };
  const previous = root.__COWBOY_NATIVE_PLUGIN_HOST;
  const calls: Array<[string, unknown]> = [];
  installPluginRuntimeHosts([descriptor], true);
  root.__COWBOY_NATIVE_PLUGIN_HOST = {
    version: PLUGIN_NATIVE_HOST_API_VERSION,
    capabilities: ["webauthn", "camera"],
    invoke: (capability, request) => {
      calls.push([capability, request]);
      return Promise.resolve({ ok: true });
    },
  };

  try {
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      true,
    );
    assertEquals(
      await supportsNativePluginCapability("passkey", "camera"),
      false,
    );
    assertEquals(
      await invokeNativePluginCapability("passkey", "webauthn", {
        action: "capabilities",
      }),
      { ok: true },
    );
    assertEquals(calls, [["webauthn", { action: "capabilities" }]]);
    await assertRejects(
      () => invokeNativePluginCapability("passkey", "camera", {}),
      Error,
      "unavailable",
    );
  } finally {
    installPluginRuntimeHosts([], true);
    if (previous === undefined) delete root.__COWBOY_NATIVE_PLUGIN_HOST;
    else root.__COWBOY_NATIVE_PLUGIN_HOST = previous;
  }
});

Deno.test("native plugin ABI rejects incompatible shell versions", async () => {
  const root = globalThis as typeof globalThis & {
    __COWBOY_NATIVE_PLUGIN_HOST?: CowboyNativePluginHost;
  };
  const previous = root.__COWBOY_NATIVE_PLUGIN_HOST;
  installPluginRuntimeHosts([descriptor], true);
  root.__COWBOY_NATIVE_PLUGIN_HOST = {
    version: "2.0.0",
    capabilities: ["webauthn"],
    invoke: () => Promise.resolve({ ok: true }),
  };
  try {
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      false,
    );
  } finally {
    installPluginRuntimeHosts([], true);
    if (previous === undefined) delete root.__COWBOY_NATIVE_PLUGIN_HOST;
    else root.__COWBOY_NATIVE_PLUGIN_HOST = previous;
  }
});

Deno.test("runtime host inventory rejects mutable and duplicate generations", async () => {
  const root = globalThis as typeof globalThis & {
    __COWBOY_NATIVE_PLUGIN_HOST?: CowboyNativePluginHost;
  };
  const previous = root.__COWBOY_NATIVE_PLUGIN_HOST;
  root.__COWBOY_NATIVE_PLUGIN_HOST = {
    version: PLUGIN_NATIVE_HOST_API_VERSION,
    capabilities: ["webauthn"],
    invoke: () => Promise.resolve({ ok: true }),
  };
  try {
    installPluginRuntimeHosts([{ ...descriptor, generation: undefined }], true);
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      false,
    );
    installPluginRuntimeHosts([{ ...descriptor, generation: "latest" }], true);
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      false,
    );
    installPluginRuntimeHosts([{
      ...descriptor,
      native_capabilities: ["webauthn", "webauthn"],
    }], true);
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      false,
    );
    installPluginRuntimeHosts(
      [{ ...descriptor, generation: "a".repeat(64) }],
      true,
    );
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      true,
    );
    installPluginRuntimeHosts([descriptor, descriptor]);
    assertEquals(
      await supportsNativePluginCapability("passkey", "webauthn"),
      false,
    );
  } finally {
    installPluginRuntimeHosts([], true);
    if (previous === undefined) delete root.__COWBOY_NATIVE_PLUGIN_HOST;
    else root.__COWBOY_NATIVE_PLUGIN_HOST = previous;
  }
});
