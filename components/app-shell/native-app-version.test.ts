import { getNativeAppVersion } from "./native-app-version.ts";

function assertEqual(actual: unknown, expected: unknown) {
  if (actual !== expected) throw new Error(`${actual} !== ${expected}`);
}

Deno.test("installed SideStore version wins over the compile-time Tauri version", async () => {
  assertEqual(
    await getNativeAppVersion({
      __cowboyNativeApp: Object.freeze({
        version: "0.1.33",
        build: "20261004020000",
      }),
      __TAURI__: { app: { getVersion: () => Promise.resolve("0.1.0") } },
    }),
    "0.1.33",
  );
});

Deno.test("legacy native shells retain their Tauri version discovery", async () => {
  assertEqual(
    await getNativeAppVersion({
      __TAURI__: { core: { invoke: () => Promise.resolve("0.1.31") } },
    }),
    "0.1.31",
  );
});

Deno.test("browser and failed or malformed bridges do not invent an installed version", async () => {
  assertEqual(await getNativeAppVersion({}), null);
  assertEqual(
    await getNativeAppVersion({ __cowboyNativeApp: { version: 33 } }),
    null,
  );
  assertEqual(
    await getNativeAppVersion({
      __TAURI__: {
        app: { getVersion: () => Promise.reject(new Error("unavailable")) },
      },
    }),
    null,
  );
});
