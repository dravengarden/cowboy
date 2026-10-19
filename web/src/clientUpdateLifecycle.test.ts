import { readFileSync } from "node:fs";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { createServiceWorkerUpdateCheck } from "./serviceWorkerUpdates";

// Exercise the production lifecycle wiring without mounting the app or
// navigating the test runner. The connection authority has its own tests.
const source = readFileSync(new URL("./main.tsx", import.meta.url), "utf8");
const lifecycle = new Bun.Transpiler({ loader: "ts" }).transformSync(
  source.slice(source.indexOf("  const installChecks ="), source.lastIndexOf("\n}")),
);

test("lifecycle signals use one authority without waiting for worker updates", async () => {
  const worker = Object.assign(new EventTarget(), {
    controller: {},
    register: async () => ({ update: () => new Promise<void>(() => {}) }),
  });
  const window = new EventTarget();
  const document = Object.assign(new EventTarget(), { visibilityState: "visible" });
  const signals: string[] = [];
  let probeSucceeded = true;
  let forcedReloads = 0;
  let now = 10_000;
  let interval!: () => void;
  const global = {
    document,
    addEventListener: window.addEventListener.bind(window),
    location: { reload: () => forcedReloads++ },
  };
  new Function(
    "globalThis", "window", "navigator", "conn",
    "createServiceWorkerUpdateCheck", "lowPriorityInterval", lifecycle,
  )(
    global, window, { serviceWorker: worker }, {
      checkForUpdate: async (trigger: string) => { signals.push(trigger); return probeSucceeded; },
    },
    (update: () => Promise<unknown>) => createServiceWorkerUpdateCheck(update, () => now),
    (check: () => void) => { interval = check; },
  );
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  window.dispatchEvent(new Event("load"));
  await settle();
  assertEquals(signals, ["periodic"]);
  for (const [target, event] of [
    [worker, "controllerchange"], [window, "focus"], [window, "online"],
    [window, "pageshow"], [window, "cowboy:native-resume"], [document, "visibilitychange"],
  ] as const) {
    now += 6_000;
    target.dispatchEvent(new Event(event));
    await settle();
    assertEquals(signals.at(-1), event);
  }
  now += 6_000;
  interval();
  await settle();
  assertEquals(signals.length, 8);
  document.visibilityState = "hidden";
  now += 6_000;
  worker.dispatchEvent(new Event("controllerchange"));
  await settle();
  assertEquals(signals.length, 8);
  assertEquals(forcedReloads, 0);
  document.visibilityState = "visible";
  probeSucceeded = false;
  window.dispatchEvent(new Event("focus"));
  await settle();
  // Restoring connectivity immediately after a failed probe must bypass the
  // five-second successful-probe dedupe window.
  probeSucceeded = true;
  now++;
  window.dispatchEvent(new Event("online"));
  await settle();
  assertEquals(signals.slice(-2), ["focus", "online"]);
  assertEquals(signals.length, 10);
});
