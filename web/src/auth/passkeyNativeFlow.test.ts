import { assertEquals, assertRejects } from "jsr:@std/assert";
import { authApi } from "./authApi.ts";
import { registerPasskey, verifyPasskey } from "./passkeyFlow.ts";
import { currentPasskeyTransports } from "./passkeyTransport.ts";
import { NativePasskeyBridgeError } from "../coreNativeBridge.ts";

for (const action of ["register", "assert"] as const) {
  Deno.test(`${action} never starts external fallback after an ambiguous native effect`, async () => {
    const priorApi = { ...authApi };
    let nativeCalls = 0;
    let externalStarts = 0;
    let completions = 0;
    const globals = {
      location: new URL("https://cowboy.example/"),
      __cowboyNativePasskeyBridgeVersion: 1,
      __cowboyNativePasskey: async (request: { action: string }) => {
        if (request.action === "capabilities") {
          return { ok: true, available: true };
        }
        nativeCalls += 1;
        throw new Error("native result lost after dispatch");
      },
      __cowboyOpenAuthenticationBrowser: () => {
        throw new Error("A second authentication browser must not open");
      },
    };
    const priorGlobals = new Map(
      Object.keys(globals).map((key) => [
        key,
        Object.getOwnPropertyDescriptor(globalThis, key),
      ]),
    );
    for (const [key, value] of Object.entries(globals)) {
      Object.defineProperty(globalThis, key, { configurable: true, value });
    }
    authApi.startPasskeyRegister = async () => ({
      challenge_id: "fixture-register",
      publicKey: {
        challenge: "challenge",
        rp: { id: "cowboy.example" },
        user: { id: "user", name: "owner" },
      },
    });
    authApi.startPasskeyAssert = async () => ({
      challenge_id: "fixture-assert",
      publicKey: { challenge: "challenge", rpId: "cowboy.example" },
    });
    authApi.completePasskeyRegister =
      authApi.completePasskeyAssert =
        async () => {
          completions += 1;
          throw new Error("No credential was returned to complete");
        };
    authApi.startExternalPasskey = async () => {
      externalStarts += 1;
      throw new Error("Unexpected external retry");
    };
    try {
      assertEquals(await currentPasskeyTransports(), ["native", "external"]);
      const error = await assertRejects(
        () =>
          action === "register" ? registerPasskey("Fixture") : verifyPasskey(),
        NativePasskeyBridgeError,
      );
      assertEquals(error.code, "outcome_unknown");
      assertEquals({ nativeCalls, externalStarts, completions }, {
        nativeCalls: 1,
        externalStarts: 0,
        completions: 0,
      });
    } finally {
      Object.assign(authApi, priorApi);
      for (const [key, descriptor] of priorGlobals) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor);
        else Reflect.deleteProperty(globalThis, key);
      }
    }
  });
}
