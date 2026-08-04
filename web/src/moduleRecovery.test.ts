import { assertEquals } from "jsr:@std/assert";
import { isModuleLoadError } from "./moduleRecovery.ts";

const recoveryGuard = await Deno.readTextFile(
  new URL("../index.html", import.meta.url),
);

Deno.test("recognizes browser module and lazy chunk load failures", () => {
  for (
    const message of [
      "Importing a module script failed.",
      "Failed to fetch dynamically imported module: /assets/DesktopApp-old.js",
      "error loading dynamically imported module",
      "Loading chunk 42 failed",
    ]
  ) {
    assertEquals(isModuleLoadError(new TypeError(message)), true, message);
  }
});

Deno.test("does not treat ordinary render failures as stale bundles", () => {
  assertEquals(isModuleLoadError(new Error("Cannot read properties of undefined")), false);
});

Deno.test("desktop pre-module recovery remains automatic after its first probe window", () => {
  assertEquals(recoveryGuard.includes("scheduleRecovery(state.attempts)"), true);
  assertEquals(recoveryGuard.includes('addEventListener("online", retryActiveRecovery)'), true);
  assertEquals(recoveryGuard.includes('addEventListener("focus", retryActiveRecovery)'), true);
  assertEquals(recoveryGuard.includes('addEventListener("pageshow", retryActiveRecovery)'), true);
  assertEquals(recoveryGuard.includes('document.visibilityState === "visible"'), true);
  assertEquals(recoveryGuard.includes("state.attempts >= 3"), false);
});

Deno.test("desktop recovery bounds stalled network probes", () => {
  assertEquals(recoveryGuard.includes("controller.abort()"), true);
  assertEquals(recoveryGuard.includes("signal: controller.signal"), true);
  assertEquals(recoveryGuard.includes("Date.now() + 12_000"), true);
});
