// The native shell's bundled loader decides, before any network request,
// whether a launch opens the app at once, probes reachability first, or tells
// the user the device is offline. The decision is a pure function between
// `@loader-plan-start` and `@loader-plan-end` markers so this test can run it
// without a browser (docs/offline-first-sync.md §Open decisions).
import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals, assertStringIncludes } from "@std/assert";

const source = await readFile(
  new URL("../apps/native-shell/loader/index.html", import.meta.url),
  "utf8",
);

function loaderPlan(): (input: {
  connectedAt?: number;
  serviceWorker: boolean;
  online: boolean;
}) => "handoff" | "probe" | "offline" {
  const start = source.indexOf("// @loader-plan-start");
  const end = source.indexOf("// @loader-plan-end");
  if (start < 0 || end < 0) throw new Error("loader plan markers are missing");
  const body = source.slice(start, end);
  return new Function(body + "\nreturn loaderPlan;")();
}

test("a shell that reached the app before opens it without probing when its WebView has Service Workers", () => {
  const plan = loaderPlan();
  // Android WebView, WebKitGTK, WebView2, iOS with App-Bound Domains: the
  // app's own worker serves the cached shell, online or not.
  assertEquals(
    plan({ connectedAt: 1_700_000_000_000, serviceWorker: true, online: true }),
    "handoff",
  );
  assertEquals(
    plan({
      connectedAt: 1_700_000_000_000,
      serviceWorker: true,
      online: false,
    }),
    "handoff",
  );
});

test("a first launch still probes, and an offline first launch says so instead of blaming permissions", () => {
  const plan = loaderPlan();
  assertEquals(plan({ serviceWorker: true, online: true }), "probe");
  assertEquals(plan({ serviceWorker: false, online: true }), "probe");
  assertEquals(plan({ serviceWorker: true, online: false }), "offline");
  assertEquals(plan({ serviceWorker: false, online: false }), "offline");
  // A WKWebView without Service Workers cannot open the app offline even
  // after a successful launch; it must not navigate into a failed load.
  assertEquals(
    plan({
      connectedAt: 1_700_000_000_000,
      serviceWorker: false,
      online: false,
    }),
    "offline",
  );
  assertEquals(
    plan({
      connectedAt: 1_700_000_000_000,
      serviceWorker: false,
      online: true,
    }),
    "probe",
  );
  // A corrupt memory never counts as a previous launch.
  assertEquals(
    plan({ connectedAt: Number.NaN, serviceWorker: true, online: true }),
    "probe",
  );
  assertEquals(
    plan({ connectedAt: 0, serviceWorker: true, online: false }),
    "offline",
  );
});

test("the loader remembers a successful hand-off and owns an offline card", () => {
  // The memory is written only after a probe succeeded, right before the
  // navigation, so an app that was never reachable never skips the probe.
  const success = source.indexOf("if (await probe()) {");
  assertStringIncludes(
    source.slice(success, success + 200),
    "rememberConnected();",
  );
  assertStringIncludes(source.slice(success, success + 200), "handOff();");
  assertStringIncludes(
    source,
    "localStorage.setItem(CONNECTED_KEY, String(Date.now()))",
  );
  // Offline presentation: its own card, the retry control, and the network
  // coming back reconnects without a tap.
  assertStringIncludes(source, '<div id="offline">');
  assertStringIncludes(source, 'body[data-state="offline"] #offline');
  assertStringIncludes(source, 'id="retry-offline"');
  assertStringIncludes(source, 'window.addEventListener("online"');
  // A hand-off that did not replace the document falls back to the probe.
  assertStringIncludes(source, "handOffFailed = true;");
  assertStringIncludes(
    source,
    "connectedAt: handOffFailed ? undefined : connectedAt()",
  );
});
