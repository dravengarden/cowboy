import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import {
  assertCoreNativePasskey,
  createCoreNativePasskey,
  hasNativePasskeyBridge,
  nativePasskeyAvailable,
  NativePasskeyBridgeError,
  nativePasskeyMayFallBack,
} from "./coreNativeBridge.ts";

const root = globalThis as typeof globalThis & {
  __cowboyNativePasskeyBridgeVersion?: unknown;
  __cowboyNativePasskey?: (request: unknown) => Promise<unknown>;
};
const creation = {
  challenge: "challenge",
  rp: { id: "cowboy.example", name: "Cowboy" },
  user: { id: "user", name: "owner", displayName: "Owner" },
};
const assertion = { challenge: "challenge", rpId: "cowboy.example" };

function success(action: "create" | "assert"): Record<string, unknown> {
  return {
    ok: true,
    credential: {
      id: "credential",
      rawId: "credential",
      type: "public-key",
      response: action === "create"
        ? { clientDataJSON: "client-data", attestationObject: "attestation" }
        : {
          clientDataJSON: "client-data",
          authenticatorData: "authenticator-data",
          signature: "signature",
          userHandle: null,
        },
      clientExtensionResults: {},
    },
  };
}

async function withPort(
  port: (request: unknown) => Promise<unknown>,
  test: () => Promise<void>,
): Promise<void> {
  const priorVersion = Object.getOwnPropertyDescriptor(
    root,
    "__cowboyNativePasskeyBridgeVersion",
  );
  const priorPort = Object.getOwnPropertyDescriptor(
    root,
    "__cowboyNativePasskey",
  );
  root.__cowboyNativePasskeyBridgeVersion = 1;
  root.__cowboyNativePasskey = port;
  try {
    await test();
  } finally {
    if (priorVersion) {
      Object.defineProperty(
        root,
        "__cowboyNativePasskeyBridgeVersion",
        priorVersion,
      );
    } else delete root.__cowboyNativePasskeyBridgeVersion;
    if (priorPort) {
      Object.defineProperty(root, "__cowboyNativePasskey", priorPort);
    } else delete root.__cowboyNativePasskey;
  }
}

Deno.test("core native Passkeys accept exactly the installed v1 ABI", async () => {
  let calls = 0;
  await withPort(async () => {
    calls += 1;
    return success("create");
  }, async () => {
    for (const version of [undefined, 0, 2, "1", Infinity, NaN]) {
      root.__cowboyNativePasskeyBridgeVersion = version;
      assertEquals(hasNativePasskeyBridge(), false);
      assertEquals(await nativePasskeyAvailable("cowboy.example"), false);
      const error = await assertRejects(
        () => createCoreNativePasskey(creation),
        NativePasskeyBridgeError,
      );
      assertEquals(error.code, "bridge_unavailable");
      assertEquals(nativePasskeyMayFallBack(error), true);
    }
    assertEquals(calls, 0);
    root.__cowboyNativePasskeyBridgeVersion = 1;
    assertEquals(hasNativePasskeyBridge(), true);
  });
});

Deno.test("core native registration and assertion have distinct owned result shapes", async () => {
  const calls: unknown[] = [];
  await withPort(async (request) => {
    calls.push(request);
    return success((request as { action: "create" | "assert" }).action);
  }, async () => {
    const created = await createCoreNativePasskey(creation);
    const verified = await assertCoreNativePasskey(assertion);
    assertEquals(created.response.attestationObject, "attestation");
    assertEquals(verified.response.signature, "signature");
    assertEquals(verified.response.userHandle, null);
    assertEquals(calls, [
      { action: "create", rp_id: "cowboy.example", public_key: creation },
      { action: "assert", rp_id: "cowboy.example", public_key: assertion },
    ]);
  });
});

Deno.test("core native decoding rejects malformed input before dispatch without coercion", async () => {
  let calls = 0;
  await withPort(async () => {
    calls += 1;
    return success("create");
  }, async () => {
    for (
      const options of [
        {},
        assertion,
        { ...creation, challenge: [] },
        { ...creation, challenge: "x".repeat(1024 * 1024 + 1) },
        { ...creation, rp: { id: { toString: () => "cowboy.example" } } },
        { ...creation, rp: { id: "cowboy.\nexample" } },
        { ...creation, user: { id: "user", name: {} } },
        { ...creation, user: [] },
      ]
    ) {
      const error = await assertRejects(
        () => createCoreNativePasskey(options),
        NativePasskeyBridgeError,
      );
      assertEquals(error.code, "invalid_request");
      assertEquals(nativePasskeyMayFallBack(error), false);
    }
    for (const options of [{}, creation, { ...assertion, rpId: 123 }]) {
      const error = await assertRejects(
        () => assertCoreNativePasskey(options),
        NativePasskeyBridgeError,
      );
      assertEquals(error.code, "invalid_request");
    }
    assertEquals(calls, 0);
  });
});

Deno.test("core native replies cannot mix success with a fallback error or smuggle fields", async () => {
  const malformed: unknown[] = [
    null,
    [],
    true,
    {},
    { ok: "true" },
    { ok: true, available: true },
    {
      ...success("create"),
      error: { code: "not_configured", message: "secret" },
    },
    {
      ...success("create"),
      ok: false,
      error: { code: "not_configured", message: "secret" },
    },
    { ok: false, error: { code: "bridge_unavailable", message: "secret" } },
    { ok: false, error: { code: "new_unknown_code", message: "secret" } },
    {
      ok: false,
      error: { code: "not_configured", message: "secret", extra: true },
    },
    success("assert"),
  ];
  const base = success("create").credential as Record<string, unknown>;
  for (
    const fields of [
      { id: "" },
      { rawId: "different" },
      { type: "other" },
      { response: [] },
      { clientExtensionResults: [] },
      { clientExtensionResults: { unexpected: "data" } },
      { extra: "secret" },
      { response: { clientDataJSON: "client-data", attestationObject: "" } },
      {
        response: {
          clientDataJSON: "bad+base64",
          attestationObject: "attestation",
        },
      },
      {
        response: {
          clientDataJSON: "client-data",
          attestationObject: "x".repeat(1024 * 1024 + 1),
        },
      },
    ]
  ) malformed.push({ ok: true, credential: { ...base, ...fields } });
  for (const reply of malformed) {
    await withPort(async () => reply, async () => {
      const error = await assertRejects(
        () => createCoreNativePasskey(creation),
        NativePasskeyBridgeError,
      );
      assertEquals(error.code, "invalid_response");
      assertEquals(nativePasskeyMayFallBack(error), false);
      assertEquals(error.message.includes("secret"), false);
    });
  }
});

Deno.test("native assertion cannot accept registration fields or malformed user handles", async () => {
  const base = success("assert").credential as Record<string, unknown>;
  const response = base.response as Record<string, unknown>;
  for (
    const badResponse of [
      {},
      [],
      { ...response, signature: 1 },
      { ...response, userHandle: [] },
      { ...response, userHandle: undefined },
      { ...response, attestationObject: "unexpected" },
    ]
  ) {
    await withPort(
      async () => ({
        ok: true,
        credential: { ...base, response: badResponse },
      }),
      async () => {
        const error = await assertRejects(
          () => assertCoreNativePasskey(assertion),
          NativePasskeyBridgeError,
        );
        assertEquals(error.code, "invalid_response");
      },
    );
  }
  await withPort(async () => ({
    ok: true,
    credential: {
      ...base,
      response: { ...response, userHandle: "user" },
    },
  }), async () => {
    assertEquals(
      (await assertCoreNativePasskey(assertion)).response.userHandle,
      "user",
    );
  });
});

Deno.test("only explicit native unavailability may fall back and error text is not reflected", async () => {
  for (
    const code of [
      "not_configured",
      "unsupported_os",
      "cancelled",
      "busy",
      "native_failure",
    ]
  ) {
    await withPort(
      async () => ({
        ok: false,
        error: { code, message: "private native payload" },
      }),
      async () => {
        const error = await assertRejects(
          () => createCoreNativePasskey(creation),
          NativePasskeyBridgeError,
        );
        assertEquals(error.code, code);
        assertEquals(
          nativePasskeyMayFallBack(error),
          code === "not_configured" || code === "unsupported_os",
        );
        assertEquals(error.message.includes("private native payload"), false);
      },
    );
  }
});

Deno.test("a lost native result never authorizes a second ceremony automatically", async () => {
  let calls = 0;
  await withPort(async () => {
    calls += 1;
    throw new Error("after native effect");
  }, async () => {
    const error = await assertRejects(
      () => createCoreNativePasskey(creation),
      NativePasskeyBridgeError,
    );
    assertEquals(error.code, "outcome_unknown");
    assertEquals(nativePasskeyMayFallBack(error), false);
    assertEquals(calls, 1);
    assertEquals(error.message.includes("after native effect"), false);
  });
});

Deno.test("core native ceremonies serialize and release their scope after completion", async () => {
  let finish!: (reply: unknown) => void;
  let calls = 0;
  const pending = new Promise<unknown>((resolve) => {
    finish = resolve;
  });
  await withPort(async () => {
    calls += 1;
    return calls === 1 ? pending : success("assert");
  }, async () => {
    const first = createCoreNativePasskey(creation);
    try {
      const error = await assertRejects(
        () => assertCoreNativePasskey(assertion),
        NativePasskeyBridgeError,
      );
      assertEquals(error.code, "busy");
      assertEquals(nativePasskeyMayFallBack(error), false);
      assertEquals(calls, 1);
    } finally {
      finish(success("create"));
      await first;
    }
    assertEquals(
      (await assertCoreNativePasskey(assertion)).response.signature,
      "signature",
    );
    assertEquals(calls, 2);
  });
});

Deno.test("a late reply cannot cross a replaced native port", async () => {
  let finish!: (reply: unknown) => void;
  const pending = new Promise<unknown>((resolve) => {
    finish = resolve;
  });
  let replacementCalls = 0;
  await withPort(async () => await pending, async () => {
    const first = createCoreNativePasskey(creation);
    root.__cowboyNativePasskey = async () => {
      replacementCalls += 1;
      return success("create");
    };
    finish(success("create"));
    const error = await assertRejects(() => first, NativePasskeyBridgeError);
    assertEquals(error.code, "outcome_unknown");
    assertEquals(nativePasskeyMayFallBack(error), false);
    assertEquals(replacementCalls, 0);
  });
});

Deno.test("capability responses have a separate closed no-effect shape", async () => {
  for (
    const reply of [[], success("create"), { ok: true, available: "true" }, {
      ok: true,
      available: true,
      extra: true,
    }]
  ) {
    await withPort(async () => reply, async () => {
      assertEquals(await nativePasskeyAvailable("cowboy.example"), false);
    });
  }
});

Deno.test("core native calls do not depend on or fetch Plugin authority", async () => {
  const source = await Deno.readTextFile(
    new URL("./coreNativeBridge.ts", import.meta.url),
  );
  for (
    const forbidden of [
      "@cowboy/plugin-api",
      "__COWBOY_NATIVE_PLUGIN_HOST",
      "fetch(",
      "pluginId",
    ]
  ) {
    assertEquals(source.includes(forbidden), false, forbidden);
  }
  assert(source.includes('action: "create"'));
  const priorFetch = globalThis.fetch;
  globalThis.fetch = () => {
    throw new Error("Catalog access is forbidden");
  };
  try {
    await withPort(async () => success("create"), async () => {
      assertEquals((await createCoreNativePasskey(creation)).id, "credential");
    });
  } finally {
    globalThis.fetch = priorFetch;
  }
});
