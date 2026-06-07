// Service worker for the cowboy PWA: makes the app installable and gives it an
// offline shell. cowboy is a LIVE ACP UI, so freshness matters more than offline
// fidelity — only the content-hashed bundle under /assets/ is cache-first (its
// filenames change on every build, so a cached copy is never stale). Everything
// else — navigations, API, AND the unhashed root files (favicon, icons,
// manifest) — is network-first, so a redeploy of those shows up immediately
// instead of being pinned to whatever the SW cached first. Bump VERSION to evict
// the old caches on the next activation.
const VERSION = "cowboy-v3";
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

  // Content-hashed bundle (vite emits hashed filenames under /assets/):
  // cache-first, populating the cache on first fetch. Safe to pin forever — a
  // new build produces new filenames.
  if (url.pathname.startsWith("/assets/")) {
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
    return;
  }

  // Everything else — navigations, API, and the unhashed root files (favicon,
  // icons, manifest): network-first. A stale transcript or a pinned old icon is
  // worse than an offline notice, so always try the network and fall back to
  // cache (and the app shell "/" for navigations) only when offline.
  event.respondWith(
    fetch(request).catch(async () => (await caches.match(request)) ?? (await caches.match("/")) ?? Response.error()),
  );
});
