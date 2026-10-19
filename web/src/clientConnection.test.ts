import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { createClientConnection } from "./clientConnection";

test("all update signals verify the loaded bundle, and a failed navigation cannot loop", async () => {
  const saved = new Map<string, PropertyDescriptor | undefined>();
  const override = (name: string, value: unknown) => {
    saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { configurable: true, value, writable: true });
  };
  let loaded = "/assets/current.js";
  let deployed = loaded;
  let offline = false;
  let invalid = false;
  let versionRequests = 0;
  const navigations: string[] = [];
  const values = new Map<string, string>();
  const doc = new EventTarget();
  Object.assign(doc, {
    querySelector: () => ({ getAttribute: () => loaded, src: `https://cowboy.test${loaded}` }),
    visibilityState: "visible",
  });
  override("document", doc);
  override("location", {
    origin: "https://cowboy.test",
    href: "https://cowboy.test/?session=kept#position",
    replace: (url: string) => navigations.push(url),
    reload: () => navigations.push("reload"),
  });
  override("navigator", {});
  override("sessionStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  });
  override("fetch", async (url: string) => {
    if (url === "/version") versionRequests++;
    if (offline) throw new Error("offline");
    return new Response(invalid ? "activation in progress" : `<script type="module" src="${deployed}"></script>`);
  });
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  try {
    const store = createClientConnection();
    assertEquals(await store.checkForUpdate(), true);
    const clean = store.watchForegroundVersion();
    store.connectionReady();
    store.connectionLost();
    store.updateAvailable();
    doc.dispatchEvent(new Event("visibilitychange"));
    await settle();
    assertEquals(versionRequests, 0);
    assertEquals(store.version(), loaded);
    assertEquals(await store.downloadUpdate(() => {}), "failed");
    assertEquals(navigations, []);
    clean();

    // A worker/foreground hint must not need a preceding /version change.
    deployed = "/assets/next.js";
    await store.checkForUpdate("controllerchange");
    assertEquals(await store.downloadUpdate(() => {}), "unsupported");
    await store.reloadIntoUpdate("unsupported");
    assertEquals(navigations.length, 1);
    const target = new URL(navigations[0]!);
    assertEquals(target.searchParams.get("session"), "kept");
    assertEquals(target.hash, "#position");
    assertEquals(target.searchParams.has("cowboy-update"), true);

    // Model WebKit replaying the old document. A new page instance shares only
    // sessionStorage, not any in-memory flag from the replaced page.
    const replayed = createClientConnection();
    await replayed.checkForUpdate();
    assertEquals(await replayed.downloadUpdate(() => {}), "rejected");
    await replayed.reloadIntoUpdate("rejected");
    assertEquals(navigations.length, 1);
    assertEquals(await replayed.downloadUpdate(() => {}, true), "unsupported");

    // Neither a failed probe nor an HTML activation page invents a target.
    loaded = deployed;
    const updated = createClientConnection();
    await updated.checkForUpdate();
    offline = true;
    assertEquals(await updated.checkForUpdate(), false);
    offline = false;
    invalid = true;
    assertEquals(await updated.checkForUpdate(), false);
    invalid = false;
    assertEquals(await updated.downloadUpdate(() => {}), "failed");
    assertEquals(navigations.length, 1);

    // A later real build can still update automatically after a failed target.
    deployed = "/assets/third.js";
    await replayed.checkForUpdate();
    assertEquals(await replayed.downloadUpdate(() => {}), "unsupported");
    await replayed.reloadIntoUpdate("unsupported");
    assertEquals(navigations.length, 2);
  } finally {
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});
