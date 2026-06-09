// Service worker for the cowboy PWA: makes the app installable and gives it an
// offline shell. cowboy is a LIVE ACP UI, so freshness matters more than offline
// fidelity — only the content-hashed bundle under /assets/ is cache-first (its
// filenames change on every build, so a cached copy is never stale). Everything
// else — navigations, API, AND the unhashed root files (favicon, icons,
// manifest) — is network-first, so a redeploy of those shows up immediately
// instead of being pinned to whatever the SW cached first. Bump VERSION to evict
// the old caches on the next activation.
// Bump on EVERY web deploy — the app's foreground update-check (main.tsx) only
// detects a new worker when this string changes, which is what triggers the
// auto-reload onto the fresh bundle.
const VERSION = "cowboy-v105";
const ASSET_CACHE = `${VERSION}-assets`;
// Immutable history pages (GET /api/history/:id/:page?v=<build>). Their content
// can never change (append-only log), and the `?v=` build token makes a new
// deploy use fresh urls — so cache-first is safe and a re-fetch (scroll back,
// reload, post-recycle) is zero-network. Version-prefixed so the activate
// cleanup evicts it on a SW update too.
const HISTORY_CACHE = `${VERSION}-history`;

self.addEventListener("install", () => {
  void self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      const stale = keys.filter((k) => !k.startsWith(VERSION));
      await Promise.all(stale.map((k) => caches.delete(k)));
      await self.clients.claim();
      // THE redeploy-reaches-the-PWA fix. An installed iOS PWA restores its old
      // page on reopen and never re-navigates, so a network-first SW alone can't
      // refresh it — but the browser DOES re-check sw.js on launch, installs a new
      // worker, and runs THIS. When this activation is an UPDATE (a prior VERSION's
      // caches existed, so it's not a first install), force every open window to
      // re-navigate onto the fresh bundle — no manual reload, no cooperation from
      // the (old) page code needed. Single reload: the reloaded page registers the
      // same worker, no new activation, no loop.
      if (stale.length > 0) {
        const wins = await self.clients.matchAll({ type: "window" });
        await Promise.all(wins.map((c) => c.navigate(c.url).catch(() => {})));
      }
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

  // Immutable history pages: cache-first (see HISTORY_CACHE). The `?v=` build
  // token guarantees a redeployed/format-changed page is a fresh url, so a
  // cached hit is never stale for the running version.
  if (url.pathname.startsWith("/api/history/")) {
    event.respondWith(
      caches.match(request).then((cached) => {
        if (cached) return cached;
        return fetch(request).then((resp) => {
          // Only cache the immutable (complete) pages — the server marks the
          // still-growing latest page `no-store`, which we must not pin.
          if (resp.ok && resp.type === "basic" && resp.headers.get("cache-control")?.includes("immutable")) {
            const copy = resp.clone();
            void caches.open(HISTORY_CACHE).then((c) => c.put(request, copy));
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
