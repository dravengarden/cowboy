import { assertEquals } from "jsr:@std/assert";
import {
  forcedBundleRecoveryUrl,
  isModuleLoadError,
  latestBundleRecoveryUrl,
} from "./moduleRecovery.ts";

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

Deno.test("mobile recovery probes the deployed entry before cache-busting navigation", async () => {
  const requests: { url: string; init?: RequestInit }[] = [];
  const recoveryUrl = await latestBundleRecoveryUrl(
    "https://cowboy.example/?session=one",
    "https://cowboy.example",
    (input, init) => {
      const url = String(input);
      requests.push({ url, init });
      if (url.includes("cowboy-recover-probe")) {
        return Promise.resolve(
          new Response(
            '<script type="module" src="/assets/index-fresh.js"></script>',
            { status: 200, headers: { "content-type": "text/html" } },
          ),
        );
      }
      return Promise.resolve(
        new Response("export {};", {
          status: 200,
          headers: { "content-type": "text/javascript" },
        }),
      );
    },
    () => 1234,
  );

  assertEquals(recoveryUrl, "https://cowboy.example/?session=one&cowboy-recover=1234");
  assertEquals(requests.map((request) => request.url), [
    "https://cowboy.example/?cowboy-recover-probe=1234",
    "https://cowboy.example/assets/index-fresh.js",
  ]);
  assertEquals(requests.every((request) => request.init?.cache === "no-store"), true);
});

Deno.test("mobile recovery stays put when the deployed entry is unavailable", async () => {
  const recoveryUrl = await latestBundleRecoveryUrl(
    "https://cowboy.example/",
    "https://cowboy.example",
    (input) =>
      Promise.resolve(
        String(input).includes("cowboy-recover-probe")
          ? new Response(
            '<script type="module" src="/assets/index-missing.js"></script>',
            { status: 200 },
          )
          : new Response("missing", { status: 404 }),
      ),
    () => 5678,
  );
  assertEquals(recoveryUrl, undefined);
});

Deno.test("manual recovery always cache-busts the current top-level URL", () => {
  assertEquals(
    forcedBundleRecoveryUrl(
      "https://cowboy.example/?session=one&cowboy-recover=old#drafts",
      () => 9012,
    ),
    "https://cowboy.example/?session=one&cowboy-recover=9012#drafts",
  );
});

Deno.test("mobile error recovery replaces the URL instead of reloading stale WKWebView HTML", async () => {
  const boundary = await Deno.readTextFile(
    new URL("./AppErrorBoundary.tsx", import.meta.url),
  );
  assertEquals(boundary.includes("latestBundleRecoveryUrl("), true);
  assertEquals(boundary.includes("if (force)"), true);
  assertEquals(boundary.includes("forcedBundleRecoveryUrl("), true);
  assertEquals(boundary.includes('markClientReloadIntent("module_error_manual_retry")'), true);
  assertEquals(boundary.includes("globalThis.location.replace(target)"), true);
  assertEquals(boundary.includes("globalThis.location.reload()"), false);
  assertEquals(boundary.includes("Checking update…"), true);
  assertEquals(boundary.includes("Retry update"), true);
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
