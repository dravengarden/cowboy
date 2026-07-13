import { assertEquals } from "jsr:@std/assert";
import { isModuleLoadError } from "./moduleRecovery.ts";

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
