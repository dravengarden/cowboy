import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import { fetchAuthStatus } from "../auth/authApi.ts";
import {
  loadProviderCatalog,
  peekProviderCatalog,
  resetProviderCatalog,
  subscribeProviderCatalog,
} from "../providerCatalogRegistry.ts";
import { providerOccupancySlot } from "../occupancyHostMap.ts";
import { providerSurfaceColor } from "../visualHostMap.ts";
import { usagePluginId } from "../usageHostMap.ts";
import { ownPluginHostLifecycle } from "./lifecycle.ts";
import { webPluginHosts } from "./inventory.ts";

function host(id = "example") {
  return {
    id,
    generation: "a".repeat(64),
    slots: ["provider.usage"],
    ui: {
      schema_version: 1,
      renderers: { "provider.usage": "provider-usage-v1" },
    },
    adapter_slot: "adapter",
    usage: { account: "account" },
    visual: {
      light: { primary: "#000000", secondary: "#ffffff" },
      dark: { primary: "#ffffff", secondary: "#000000" },
    },
  };
}
function catalog(hosts = [host()]) {
  return {
    providers: [],
    authentications: [],
    authentication_executors: [],
    platform: { hosts },
  };
}
function authentication(id = "identity-source") {
  return {
    registration: {
      enabled: false,
      mode: "disabled",
      accepts_registration: false,
    },
    host_plugins: [{
      id,
      generation: "a".repeat(64),
      slots: ["login.method"],
      ui: { schema_version: 1, renderers: { "login.method": "login-oidc-v1" } },
    }],
  };
}
function clean(): void {
  webPluginHosts.reset();
  resetProviderCatalog();
}

Deno.test("the Web root explicitly owns and releases its single session-end listener", () => {
  const target = new EventTarget();
  let resets = 0;
  const dispose = ownPluginHostLifecycle(target, () => resets++);
  target.dispatchEvent(new Event("cowboy:product-sign-out"));
  assertEquals(resets, 1);
  dispose();
  dispose();
  assertEquals(resets, 2);
  target.dispatchEvent(new Event("cowboy:product-sign-out"));
  assertEquals(resets, 2);
});

Deno.test("Catalog validates the envelope before changing any presentation projection", async () => {
  const previous = globalThis.fetch;
  clean();
  try {
    globalThis.fetch = () => Promise.resolve(Response.json(catalog()));
    await loadProviderCatalog();
    const accepted = peekProviderCatalog();
    assertEquals(
      webPluginHosts.resolve("example", "provider.usage").kind,
      "ready",
    );
    assertEquals(providerOccupancySlot("example"), "adapter");
    assertEquals(usagePluginId("account"), "example");
    assert(providerSurfaceColor("example"));
    globalThis.fetch = () =>
      Promise.resolve(
        Response.json({ ...catalog([host("wrong")]), providers: "bad" }),
      );
    await assertRejects(
      () => loadProviderCatalog(true),
      Error,
      "Invalid Provider Catalog response",
    );
    assertEquals(peekProviderCatalog(), accepted);
    assertEquals(providerOccupancySlot("example"), "adapter");
    assertEquals(usagePluginId("account"), "example");
    assertEquals(
      webPluginHosts.resolve("wrong", "provider.usage").kind,
      "missing",
    );
  } finally {
    globalThis.fetch = previous;
    clean();
  }
});

Deno.test("session end clears all Web observations; a late Catalog response cannot restore them", async () => {
  const previous = globalThis.fetch;
  const target = new EventTarget();
  const dispose = ownPluginHostLifecycle(target);
  clean();
  try {
    globalThis.fetch = () => Promise.resolve(Response.json(catalog()));
    await loadProviderCatalog();
    const delayed = Promise.withResolvers<Response>();
    let signal: AbortSignal | undefined;
    globalThis.fetch = (_input, init) => {
      signal = init?.signal ?? undefined;
      return delayed.promise;
    };
    const pending = loadProviderCatalog(true);
    const rejected = assertRejects(() => pending, Error, "superseded");
    target.dispatchEvent(new Event("cowboy:product-sign-out"));
    assert(signal?.aborted);
    assertEquals(peekProviderCatalog(), null);
    assertEquals(providerOccupancySlot("example"), undefined);
    assertEquals(providerSurfaceColor("example"), undefined);
    assertEquals(usagePluginId("account"), "account");
    delayed.resolve(Response.json(catalog()));
    await rejected;
    assertEquals(peekProviderCatalog(), null);
    assertEquals(
      webPluginHosts.resolve("example", "provider.usage").kind,
      "pending",
    );
  } finally {
    dispose();
    globalThis.fetch = previous;
    clean();
  }
});

Deno.test("an old Catalog finally cannot clear a new session's pending read", async () => {
  const previous = globalThis.fetch;
  clean();
  const old = Promise.withResolvers<Response>();
  const current = Promise.withResolvers<Response>();
  let requests = 0;
  globalThis.fetch = () => ++requests === 1 ? old.promise : current.promise;
  try {
    const first = loadProviderCatalog(true);
    const rejected = assertRejects(() => first, Error, "superseded");
    clean();
    const second = loadProviderCatalog(true);
    old.resolve(Response.json(catalog([host("old")])));
    await rejected;
    const joined = loadProviderCatalog(true);
    assertEquals(requests, 2);
    current.resolve(Response.json(catalog([host("current")])));
    await Promise.all([second, joined]);
    assertEquals(
      webPluginHosts.resolve("old", "provider.usage").kind,
      "missing",
    );
    assertEquals(
      webPluginHosts.resolve("current", "provider.usage").kind,
      "ready",
    );
  } finally {
    globalThis.fetch = previous;
    clean();
  }
});

Deno.test("Catalog subscriptions own their registrations even with the same callback", async () => {
  const previous = globalThis.fetch;
  clean();
  let calls = 0;
  const callback = () => calls++;
  const first = subscribeProviderCatalog(callback);
  const second = subscribeProviderCatalog(callback);
  globalThis.fetch = () => Promise.resolve(Response.json(catalog()));
  try {
    await loadProviderCatalog(true);
    assertEquals(calls, 2);
    first();
    first();
    await loadProviderCatalog(true);
    assertEquals(calls, 3);
  } finally {
    second();
    globalThis.fetch = previous;
    clean();
  }
});

Deno.test("auth observations reject old responses and do not resurrect hosts across session end", async () => {
  const previous = globalThis.fetch;
  clean();
  const old = Promise.withResolvers<Response>();
  const current = Promise.withResolvers<Response>();
  let requests = 0;
  const signals: (AbortSignal | null | undefined)[] = [];
  globalThis.fetch = (_input, init) => {
    signals.push(init?.signal);
    return ++requests === 1 ? old.promise : current.promise;
  };
  try {
    const first = fetchAuthStatus();
    const second = fetchAuthStatus();
    assert(signals[0]?.aborted);
    current.resolve(Response.json(authentication("current")));
    assertEquals((await second).kind, "ok");
    old.resolve(Response.json(authentication("old")));
    assertEquals((await first).kind, "network");
    assertEquals(webPluginHosts.resolve("old", "login.method").kind, "missing");
    assertEquals(
      webPluginHosts.resolve("current", "login.method").kind,
      "ready",
    );
    const late = Promise.withResolvers<Response>();
    globalThis.fetch = () => late.promise;
    const third = fetchAuthStatus();
    clean();
    late.resolve(Response.json(authentication("late")));
    assertEquals((await third).kind, "network");
    assertEquals(
      webPluginHosts.resolve("late", "login.method").kind,
      "pending",
    );
  } finally {
    globalThis.fetch = previous;
    clean();
  }
});

Deno.test("invalid auth status never installs its host rows; explicit empty inventory removes old rows", async () => {
  const previous = globalThis.fetch;
  clean();
  try {
    globalThis.fetch = () => Promise.resolve(Response.json(authentication()));
    assertEquals((await fetchAuthStatus()).kind, "ok");
    globalThis.fetch = () =>
      Promise.resolve(
        Response.json({ ...authentication("bad"), registration: null }),
      );
    assertEquals((await fetchAuthStatus()).kind, "unsupported");
    assertEquals(webPluginHosts.resolve("bad", "login.method").kind, "missing");
    assertEquals(
      webPluginHosts.resolve("identity-source", "login.method").kind,
      "ready",
    );
    globalThis.fetch = () =>
      Promise.resolve(Response.json({ ...authentication(), host_plugins: [] }));
    assertEquals((await fetchAuthStatus()).kind, "ok");
    assertEquals(
      webPluginHosts.resolve("identity-source", "login.method").kind,
      "missing",
    );
  } finally {
    globalThis.fetch = previous;
    clean();
  }
});
