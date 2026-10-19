import { readFileSync } from "node:fs";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  checkForDeployedUpdate,
  createServiceWorkerUpdateCheck,
} from "./serviceWorkerUpdates";

// Exercise the actual production lifecycle wiring without mounting the app,
// connecting a session, or navigating the test runner.
const source = readFileSync(new URL("./main.tsx", import.meta.url), "utf8");
const lifecycle = new Bun.Transpiler({ loader: "ts" }).transformSync(
  source.slice(source.indexOf("  const installChecks ="), source.lastIndexOf("\n}")),
);

test("worker claims verify the deployed build before announcing any reload", async () => {
  const worker = new EventTarget() as EventTarget & {
    controller: object;
    register: () => Promise<{ update: () => Promise<void> }>;
  };
  worker.controller = {};
  worker.register = async () => ({ update: async () => {} });
  const window = new EventTarget();
  const document = new EventTarget() as EventTarget & { visibilityState: string };
  document.visibilityState = "visible";
  let entry = "/assets/current.js";
  let offline = false;
  let reports = 0;
  let forcedReloads = 0;
  let now = 10_000;
  let interval!: () => void;
  const global = {
    document,
    addEventListener: window.addEventListener.bind(window),
    location: { reload: () => forcedReloads++ },
    fetch: async () => {
      if (offline) throw new Error("offline");
      return new Response(`<script type="module" src="${entry}"></script>`);
    },
  };
  new Function(
    "globalThis", "window", "navigator", "loadedEntry", "reportUpdate",
    "createServiceWorkerUpdateCheck", "checkForDeployedUpdate",
    "lowPriorityInterval", "markClientReloadIntent", lifecycle,
  )(
    global, window, { serviceWorker: worker }, "/assets/current.js",
    async () => { reports++; },
    (update: () => Promise<unknown>) => createServiceWorkerUpdateCheck(update, () => now),
    checkForDeployedUpdate,
    (check: () => void) => { interval = check; },
    () => {},
  );
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  window.dispatchEvent(new Event("load"));
  await settle();
  assertEquals(reports, 0);
  // A delayed claim can occur after an already-current page starts, and again
  // after an imported worker dependency changes without a new app build.
  for (let index = 0; index < 2; index++) {
    now += 6_000;
    worker.dispatchEvent(new Event("controllerchange"));
    await settle();
    assertEquals(reports, 0);
    assertEquals(forcedReloads, 0);
  }
  offline = true;
  now += 6_000;
  worker.dispatchEvent(new Event("controllerchange"));
  await settle();
  assertEquals(forcedReloads, 0);
  // A failed check stays retryable: the periodic check still discovers a real
  // deployment as soon as the network comes back.
  offline = false;
  entry = "/assets/next.js";
  interval();
  await settle();
  assertEquals(reports, 1);
});
