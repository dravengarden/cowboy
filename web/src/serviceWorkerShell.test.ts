// Executable contract for the service worker's app shell
// (docs/offline-first-sync.md §Boot). The worker is plain JS, so it is
// evaluated here against in-memory fakes of `caches`, `fetch` and `self`.
import { assert, assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(new URL("../public/sw.js", import.meta.url));
const version = /const VERSION = "(cowboy-v\d+)"/.exec(source)![1]!;
const SHELL = `${version}-shell`;
const ASSETS = `${version}-assets`;
const ORIGIN = "https://cowboy.test";

function path(request: string | { url: string }): string {
  if (typeof request === "string") return request;
  const url = new URL(request.url);
  return url.pathname + url.search;
}

class FakeCache {
  readonly entries = new Map<string, Response>();
  put(request: string | { url: string }, response: Response): Promise<void> {
    this.entries.set(path(request), response);
    return Promise.resolve();
  }
  match(request: string | { url: string }): Promise<Response | undefined> {
    return Promise.resolve(this.entries.get(path(request))?.clone());
  }
}

class FakeCaches {
  readonly byName = new Map<string, FakeCache>();
  open(name: string): Promise<FakeCache> {
    let cache = this.byName.get(name);
    if (!cache) {
      cache = new FakeCache();
      this.byName.set(name, cache);
    }
    return Promise.resolve(cache);
  }
  keys(): Promise<string[]> {
    return Promise.resolve([...this.byName.keys()]);
  }
  async match(
    request: string | { url: string },
    options?: { cacheName?: string },
  ): Promise<Response | undefined> {
    if (options?.cacheName !== undefined) {
      return await this.byName.get(options.cacheName)?.match(request);
    }
    for (const cache of this.byName.values()) {
      const hit = await cache.match(request);
      if (hit) return hit;
    }
    return undefined;
  }
  delete(name: string): Promise<boolean> {
    return Promise.resolve(this.byName.delete(name));
  }
}

function basic(body: string, status = 200): Response {
  const response = new Response(body, { status });
  Object.defineProperty(response, "type", { value: "basic" });
  return response;
}

function shellHtml(entry: string, boot: readonly string[]): string {
  return `<html><head><script type="application/json" id="cowboy-boot-assets">${
    JSON.stringify(boot)
  }</script></head><body><script type="module" src="${entry}"></script></body></html>`;
}

interface Worker {
  readonly caches: FakeCaches;
  readonly fetched: string[];
  network: (url: string) => Promise<Response>;
  navigate(url: string): { response: Promise<Response>; settled: () => Promise<unknown> };
  message(data: unknown): Promise<unknown>;
}

function startWorker(): Worker {
  const listeners = new Map<string, (event: unknown) => void>();
  const caches = new FakeCaches();
  const fetched: string[] = [];
  const worker: Worker = {
    caches,
    fetched,
    network: () => Promise.reject(new TypeError("offline")),
    navigate(url) {
      let response: Promise<Response> | undefined;
      const waits: Promise<unknown>[] = [];
      listeners.get("fetch")!({
        request: { method: "GET", url: ORIGIN + url, mode: "navigate" },
        respondWith: (value: Promise<Response> | Response) => {
          response = Promise.resolve(value);
        },
        waitUntil: (value: Promise<unknown>) => waits.push(value),
      });
      if (response === undefined) throw new Error("the worker did not answer " + url);
      return { response, settled: () => Promise.all(waits) };
    },
    message(data) {
      return new Promise((resolve) => {
        const waits: Promise<unknown>[] = [];
        listeners.get("message")!({
          data,
          ports: [{ postMessage: resolve }],
          waitUntil: (value: Promise<unknown>) => waits.push(value),
        });
      });
    },
  };
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type: string, listener: (event: unknown) => void) =>
      listeners.set(type, listener),
    skipWaiting: () => Promise.resolve(),
    clients: { claim: () => Promise.resolve(), matchAll: () => Promise.resolve([]) },
    registration: { showNotification: () => Promise.resolve() },
  };
  const fetchFake = (request: string | { url: string }): Promise<Response> => {
    const url = path(request);
    fetched.push(url);
    return worker.network(url);
  };
  new Function("self", "caches", "fetch", source)(self, caches, fetchFake);
  return worker;
}

async function cachedText(caches: FakeCaches, name: string, key: string): Promise<string | undefined> {
  return await (await caches.match(key, { cacheName: name }))?.text();
}

Deno.test("a cached shell answers a navigation while the network is still hanging", async () => {
  const worker = startWorker();
  await (await worker.caches.open(SHELL)).put("/", basic("cached shell"));
  // A weak connection is slow, not failed: this request never settles.
  worker.network = () => new Promise(() => undefined);
  const { response } = worker.navigate("/?session=abc");
  const winner = await Promise.race([
    response.then((value) => value.text()),
    new Promise<string>((resolve) => setTimeout(() => resolve("waited for the network"), 50)),
  ]);
  assertEquals(winner, "cached shell");
});

Deno.test("the deployed shell is promoted only once its boot assets are cached", async () => {
  const worker = startWorker();
  await (await worker.caches.open(SHELL)).put("/", basic("old shell"));
  const deployed = shellHtml("/assets/main-new.js", ["/assets/MobileApp-new.js"]);
  let chunkReady = false;
  worker.network = (url) => {
    if (url === "/") return Promise.resolve(basic(deployed));
    if (url === "/assets/main-new.js") return Promise.resolve(basic("entry"));
    if (url === "/assets/MobileApp-new.js") {
      return Promise.resolve(chunkReady ? basic("surface") : basic("missing", 404));
    }
    return Promise.reject(new TypeError("unexpected " + url));
  };

  const first = worker.navigate("/");
  assertEquals(await (await first.response).text(), "old shell");
  await first.settled();
  // One boot asset failed: the old shell must stay, or the next launch would
  // open a document whose surface chunk still needs the network.
  assertEquals(await cachedText(worker.caches, SHELL, "/"), "old shell");

  chunkReady = true;
  const second = worker.navigate("/");
  assertEquals(await (await second.response).text(), "old shell");
  await second.settled();
  assertEquals(await cachedText(worker.caches, SHELL, "/"), deployed);
  assertEquals(await cachedText(worker.caches, ASSETS, "/assets/main-new.js"), "entry");
  assertEquals(await cachedText(worker.caches, ASSETS, "/assets/MobileApp-new.js"), "surface");

  // The launch after the deploy boots the new build entirely from cache.
  worker.network = () => new Promise(() => undefined);
  assertEquals(await (await worker.navigate("/").response).text(), deployed);
});

Deno.test("a new worker generation opens from the previous generation's shell", async () => {
  const worker = startWorker();
  await (await worker.caches.open("cowboy-v7-shell")).put("/", basic("older"));
  await (await worker.caches.open("cowboy-v9-shell")).put("/", basic("previous"));
  worker.network = () => new Promise(() => undefined);
  assertEquals(await (await worker.navigate("/").response).text(), "previous");
});

Deno.test("a device with no shell, an update and a recovery all go to the network", async () => {
  const worker = startWorker();
  worker.network = (url) =>
    url.startsWith("/?") || url === "/"
      ? Promise.resolve(basic("deployed"))
      : Promise.reject(new TypeError("unexpected " + url));
  const cold = worker.navigate("/");
  assertEquals(await (await cold.response).text(), "deployed");
  await cold.settled();
  assertEquals(await cachedText(worker.caches, SHELL, "/"), "deployed");

  await (await worker.caches.open(SHELL)).put("/", basic("cached shell"));
  for (const url of ["/?cowboy-update=1", "/?cowboy-recover=1"]) {
    assertEquals(await (await worker.navigate(url).response).text(), "deployed");
  }
  // Without a network those explicit requests still fall back to the cache.
  worker.network = () => Promise.reject(new TypeError("offline"));
  assertEquals(await (await worker.navigate("/?cowboy-update=2").response).text(), "deployed");
});

Deno.test("the update action learns whether the deployed shell was downloaded", async () => {
  const worker = startWorker();
  worker.network = () => Promise.reject(new TypeError("offline"));
  assertEquals(await worker.message({ type: "cowboy.refresh-shell" }), { ok: false });
  worker.network = (url) =>
    Promise.resolve(basic(url === "/" ? shellHtml("/assets/main.js", []) : "asset"));
  assertEquals(await worker.message({ type: "cowboy.refresh-shell" }), { ok: true });
  assert((await cachedText(worker.caches, SHELL, "/"))?.includes("/assets/main.js"));
});

Deno.test("admin, passkey and identity requests never touch the shell cache", async () => {
  const worker = startWorker();
  await (await worker.caches.open(SHELL)).put("/", basic("cached shell"));
  worker.network = (url) => Promise.resolve(basic("network " + url));
  for (const url of ["/admin", "/admin/users", "/passkey.html"]) {
    assertEquals(await (await worker.navigate(url).response).text(), "network " + url);
  }
  assertEquals(await cachedText(worker.caches, SHELL, "/"), "cached shell");
});
