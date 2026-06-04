// Service worker for the cowboy PWA: makes the app installable and gives it an
// offline shell. cowboy is a LIVE ACP UI, so freshness matters more than offline
// fidelity — navigations + API are network-first (fall back to cache only when
// offline); content-hashed static assets are cache-first. Bump VERSION to evict
// the old caches on the next activation.
const VERSION = "cowboy-v1";
const ASSET_CACHE = `${VERSION}-assets`;

self.addEventListener("install", () => {
  void self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((k) => !k.startsWith(VERSION)).map((k) => caches.delete(k)));
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  // Navigations + API: network-first. A stale transcript is worse than an
  // offline notice, so always try the network and fall back to cache (and to
  // the app shell "/" for navigations) only when it fails.
  if (request.mode === "navigate" || url.pathname.startsWith("/api/")) {
    event.respondWith(
      fetch(request).catch(async () => (await caches.match(request)) ?? (await caches.match("/")) ?? Response.error()),
    );
    return;
  }

  // Hashed static assets (vite emits content-hashed filenames): cache-first,
  // populating the cache on first fetch.
  event.respondWith(
    caches.match(request).then((cached) => {
      if (cached) return cached;
      return fetch(request).then((resp) => {
        if (resp.ok && resp.type === "basic") {
          const copy = resp.clone();
          void caches.open(ASSET_CACHE).then((c) => c.put(request, copy));
        }
        return resp;
      });
    }),
  );
});
