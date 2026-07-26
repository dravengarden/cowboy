// Service worker for the cowboy PWA: makes the app installable and gives it an
// offline shell. cowboy is a LIVE ACP UI, so freshness matters more than offline
// fidelity — only the content-hashed bundle under /assets/ is cache-first (its
// filenames change on every build, so a cached copy is never stale). Everything
// else — navigations, API, AND the unhashed root files (favicon, icons,
// manifest) — is network-first, so a redeploy of those shows up immediately
// instead of being pinned to whatever the SW cached first. Bump VERSION to evict
// the old caches on the next activation.
// Bump on EVERY web deploy — the app's foreground update-check (main.tsx) only
// detects a new worker when this string changes. Desktop auto-reloads after its
// visible countdown; Mobile waits for an explicit Update tap.
const VERSION = "cowboy-v739";
const ASSET_CACHE = `${VERSION}-assets`;
// The app shell ("/" — index.html). Cowboy serves the independently switched
// frontend on the same origin as the API/WS, so when the daemon is down (e.g. a
// nixos-rebuild and not yet restarted) a navigation to "/" gets nothing and the
// PWA shows a blank white page. We cache the shell on every successful navigation
// so an offline reopen (or a backend outage) still loads the cached shell + the
// cache-first /assets bundle, which then renders the app's OWN reconnect banner
// instead of white. Network-first stays — the cache is a fallback only, never
// pinned over a reachable daemon, so a redeploy is still picked up immediately.
const SHELL_CACHE = `${VERSION}-shell`;
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
      // Keep the immediately previous Cowboy cache generation as a rolling
      // hand-off. An already-open client can request one of its old lazy chunks
      // in the short interval between claim() and controllerchange reload. If
      // activation deletes that cache first, the independently switched web
      // root no longer contains the old hash and the dynamic import crashes.
      // Two generations bound storage while making web-only deploys atomic from
      // every open client's point of view. Never delete another app's caches.
      const cowboyVersions = [...new Set(keys.flatMap((key) => {
        const match = /^cowboy-v(\d+)-/.exec(key);
        return match ? [Number(match[1])] : [];
      }))].sort((a, b) => b - a);
      const keep = new Set(cowboyVersions.slice(0, 2));
      const stale = keys.filter((key) => {
        const match = /^cowboy-v(\d+)-/.exec(key);
        return match != null && !keep.has(Number(match[1]));
      });
      await Promise.all(stale.map((k) => caches.delete(k)));
      await self.clients.claim();
      // `clients.claim()` changes the active controller for every open window.
      // main.tsx owns the ONE resulting navigation via its `controllerchange`
      // handler. Do not also call `client.navigate()` here: the two navigations
      // race, which made desktop shells intermittently appear frozen during a
      // web-only deploy. Installed PWAs still refresh automatically because they
      // re-check sw.js on launch/foreground and surface an update once.
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

  // The optional mvdan/sh formatter is a lazy, versioned application asset.
  // Cache it after first use so the relatively large WASM parser is paid for
  // once per release, never on startup and never on every Tool details open.
  if (url.pathname === "/shellfmt.wasm" || url.pathname === "/wasm_exec.js") {
    event.respondWith(
      caches.match(request).then((cached) => {
        if (cached) return cached;
        return fetch(request).then((resp) => {
          if (resp.ok && resp.type === "basic") {
            const copy = resp.clone();
            void caches.open(ASSET_CACHE).then((cache) => cache.put(request, copy));
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

  // Navigations: network-first, but populate SHELL_CACHE on every success so the
  // offline fallback below actually has a shell to serve. Without this the
  // `caches.match("/")` fallback was dead code (nothing ever cached "/"), so a
  // dead daemon = blank white page instead of the cached shell + reconnect UI.
  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request)
        .then((resp) => {
          if (resp.ok && resp.type === "basic") {
            const copy = resp.clone();
            void caches.open(SHELL_CACHE).then((c) => c.put("/", copy));
          }
          return resp;
        })
        .catch(async () => (await caches.match("/", { cacheName: SHELL_CACHE })) ?? Response.error()),
    );
    return;
  }

  // Everything else — API and the unhashed root files (favicon, icons,
  // manifest): network-first. A stale transcript or a pinned old icon is
  // worse than an offline notice, so always try the network and fall back to
  // cache only when offline.
  event.respondWith(
    fetch(request).catch(async () => (await caches.match(request)) ?? Response.error()),
  );
});
