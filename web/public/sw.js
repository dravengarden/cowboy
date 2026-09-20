// Service worker for the cowboy PWA: makes the app installable and opens it
// from this device, not from the network (docs/offline-first-sync.md §Boot).
//   - The app shell ("/") is CACHE-FIRST and refreshed in the background. A
//     weak connection is slow, not failed, so a network-first shell left the
//     installed PWA on a white page for as long as the request hung. A new
//     shell is promoted only after its boot assets are cached, so the launch
//     after a deploy is as instant as any other; the page's own build probe
//     (main.tsx) still detects the new build and offers the update.
//   - The content-hashed bundle under /assets/ is cache-first (its filenames
//     change on every build, so a cached copy is never stale).
//   - API and the unhashed root files (favicon, icons, manifest) stay
//     network-first: a stale transcript or a pinned old icon is worse than an
//     offline notice.
// Bump VERSION to evict the old caches on the next activation.
// Bump on EVERY web deploy — the app's foreground update-check (main.tsx) only
// detects a new worker when this string changes. Every surface downloads the
// deployed build as soon as it is detected, then reloads itself after a visible
// countdown once its user is idle; a press only brings that reload forward.
const VERSION = "cowboy-v1756";
const ASSET_CACHE = `${VERSION}-assets`;
// The app shell ("/" — index.html). Served from here first; see the header.
// A redeploy is never pinned away: every launch refreshes this cache in the
// background, and the page compares its loaded entry with the deployed index
// over the network to surface the update.
const SHELL_CACHE = `${VERSION}-shell`;
// Immutable history pages (GET /api/history/:id/:page?v=<build>). Their content
// can never change (append-only log), and the `?v=` build token makes a new
// deploy use fresh urls — so cache-first is safe and a re-fetch (scroll back,
// reload, post-recycle) is zero-network. Version-prefixed so the activate
// cleanup evicts it on a SW update too.
const HISTORY_CACHE = `${VERSION}-history`;
// This generation's own notes. Version-scoped on purpose: the next deploy is a
// new VERSION with a fresh, empty state cache, so a note about a build can
// never outlive the build it was about.
const STATE_CACHE = `${VERSION}-state`;
// The deployed build was rolled back on this device because it did not start.
// The body is the number of times that has happened.
const ROLLBACK_MARK = "/cowboy-rolled-back";

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

const NOTIFICATION_CATEGORIES = new Set(["completed", "input", "permission", "error"]);
const SAFE_SESSION_ID = /^[A-Za-z0-9_-]{1,160}$/;
const ACTIVE_SESSIONS = new Map();

function validNotificationMessage(message) {
  return Boolean(
    message && typeof message === "object" &&
    message.version === 1 &&
    NOTIFICATION_CATEGORIES.has(message.category) &&
    SAFE_SESSION_ID.test(message.sessionId ?? "") &&
    typeof message.title === "string" && message.title.length <= 120 &&
    typeof message.body === "string" && message.body.length <= 240
  );
}

function showSessionNotification(message) {
  const url = message.test === true ? "/" : `/?session=${encodeURIComponent(message.sessionId)}`;
  return self.registration.showNotification(message.title, {
    body: message.body,
    icon: "/cowboy-app-icon-192-v10.png",
    badge: "/cowboy-app-icon-192-v10.png",
    tag: `cowboy-session-${message.sessionId}`,
    data: { url, sessionId: message.test === true ? null : message.sessionId },
  });
}

// --- App shell: cache-first, refreshed in the background ---------------------
// Navigations that ask for the deployed build explicitly (Desktop module
// recovery, an applied update) go to the network first.
const SHELL_NETWORK_PARAMS = ["cowboy-recover", "cowboy-update"];
const BOOT_ASSET_FETCHES = 6;
let shellRefresh;
// Pages watching the refresh that is running now. The update control fills from
// this count, so every client sees one download — including a page that joined
// the refresh a navigation had already started, which is told the current count
// at once instead of waiting for the next batch.
const shellProgressPorts = new Set();
let shellProgress;

function publishShellProgress(done, total) {
  shellProgress = { type: "progress", done, total };
  for (const port of shellProgressPorts) {
    try {
      port.postMessage(shellProgress);
    } catch {
      shellProgressPorts.delete(port);
    }
  }
}

async function rollbackAttempts() {
  const hit = await caches.match(ROLLBACK_MARK, { cacheName: STATE_CACHE });
  if (!hit) return 0;
  const count = Number(await hit.text());
  return Number.isFinite(count) && count > 0 ? count : 1;
}

// Serve the newest generation that is not this one, and stop promoting the
// deployed shell until someone asks again. The two-generation retention above
// is what makes this possible: the build the user was running moments ago is
// still whole in cache, document and hashed assets alike.
async function rollbackShell() {
  const keys = await caches.keys();
  const generations = keys
    .filter((key) => /^cowboy-v\d+-shell$/.test(key) && key !== SHELL_CACHE)
    .sort((a, b) => Number(/\d+/.exec(b)[0]) - Number(/\d+/.exec(a)[0]));
  for (const key of generations) {
    const previous = await caches.match("/", { cacheName: key });
    if (!previous) continue;
    await (await caches.open(SHELL_CACHE)).put("/", previous.clone());
    const attempts = (await rollbackAttempts()) + 1;
    await (await caches.open(STATE_CACHE)).put(ROLLBACK_MARK, new Response(String(attempts)));
    return { ok: true, version: /cowboy-v\d+/.exec(key)[0], attempts };
  }
  // Nothing to fall back to. The caller keeps running whatever it has.
  return { ok: false, attempts: await rollbackAttempts() };
}

async function cachedShell() {
  const current = await caches.match("/", { cacheName: SHELL_CACHE });
  if (current) return current;
  // A new worker generation starts with an empty shell cache. Until its first
  // refresh lands, the previous generation's shell still opens the app at
  // once: activation keeps that generation's hashed assets too.
  const generations = (await caches.keys())
    .filter((key) => /^cowboy-v\d+-shell$/.test(key) && key !== SHELL_CACHE)
    .sort((a, b) => Number(/\d+/.exec(b)[0]) - Number(/\d+/.exec(a)[0]));
  for (const key of generations) {
    const hit = await caches.match("/", { cacheName: key });
    if (hit) return hit;
  }
  return undefined;
}

// Everything a shell needs before the app can paint: the build-emitted boot
// closure (vite.config.ts `cowboy-boot-assets`, both surfaces) plus whatever
// the document itself references under /assets/.
function bootAssetUrls(html) {
  const urls = new Set();
  const block = /<script[^>]*\bid="cowboy-boot-assets"[^>]*>([\s\S]*?)<\/script>/.exec(html);
  if (block) {
    try {
      for (const url of JSON.parse(block[1])) {
        if (typeof url === "string" && url.startsWith("/assets/")) urls.add(url);
      }
    } catch {
      // A malformed list only loses the precache; the document scan remains.
    }
  }
  for (const match of html.matchAll(/<(?:script|link)\b[^>]*\b(?:src|href)="(\/assets\/[^"]+)"/g)) {
    urls.add(match[1]);
  }
  return [...urls];
}

// Fetch the deployed shell and cache its boot assets; promote it only when the
// whole set is here, so a cached shell can always boot without the network.
function refreshShell() {
  shellRefresh ??= (async () => {
    try {
      // A device that rolled back has already watched this build fail to
      // start. Fetching it again would quietly put it back as the shell.
      if (await rollbackAttempts() > 0) return false;
      const response = await fetch("/", { cache: "no-store", credentials: "same-origin" });
      if (!response.ok || response.type !== "basic") return false;
      const html = await response.clone().text();
      const urls = bootAssetUrls(html);
      const assets = await caches.open(ASSET_CACHE);
      publishShellProgress(0, urls.length);
      for (let index = 0; index < urls.length; index += BOOT_ASSET_FETCHES) {
        const batch = await Promise.all(
          urls.slice(index, index + BOOT_ASSET_FETCHES).map(async (url) => {
            if (await caches.match(url)) return true;
            const asset = await fetch(url, { credentials: "same-origin" });
            if (!asset.ok || asset.type !== "basic") return false;
            await assets.put(url, asset);
            return true;
          }),
        );
        if (!batch.every(Boolean)) return false;
        publishShellProgress(Math.min(index + BOOT_ASSET_FETCHES, urls.length), urls.length);
      }
      // Re-checked: a rollback can land while this refresh is in flight, and
      // promoting here would undo it.
      if (await rollbackAttempts() > 0) return false;
      await (await caches.open(SHELL_CACHE)).put("/", response);
      return true;
    } catch {
      return false;
    }
  })().finally(() => {
    shellRefresh = undefined;
    shellProgress = undefined;
  });
  return shellRefresh;
}

self.addEventListener("message", (event) => {
  const message = event.data;
  // The page asks for the deployed shell the moment it detects a deploy, long
  // before it reloads: the reload then boots the new build from cache instead of
  // racing the network, and the control that brings it forward can promise that.
  // Progress is OPT-IN, and it has to stay that way. This reply port's consumer
  // is the PREVIOUS build's client — the one asking to be replaced — and that
  // client resolves on the first message it receives, reading anything without
  // `ok` as a failed download. Streaming progress at it unasked strands it on
  // "could not be downloaded yet" for the rest of its life, retrying every
  // minute against a download that in fact succeeded. So the legacy request
  // keeps its exact one-message contract, and only a client that asks by
  // sending `progress: true` is told about batches. Never widen what an
  // unflagged `cowboy.refresh-shell` sends back.
  if (message?.type === "cowboy.refresh-shell") {
    const port = event.ports?.[0];
    const watching = port && message.progress === true;
    event.waitUntil((async () => {
      // A press means "try that again", so it is also what lifts a rollback.
      if (message.retry === true) {
        await (await caches.open(STATE_CACHE)).delete(ROLLBACK_MARK);
      }
      const rejected = await rollbackAttempts();
      if (rejected > 0) {
        // Still exactly one message, and still shaped for the previous
        // build: it reads the missing `ok` as "not downloaded", which is true.
        port?.postMessage({ ok: false, rolledBack: true, attempts: rejected });
        return;
      }
      if (watching) {
        shellProgressPorts.add(port);
        if (shellProgress) port.postMessage(shellProgress);
      }
      const ok = await refreshShell();
      if (!port) return;
      if (watching) shellProgressPorts.delete(port);
      port.postMessage({ ok });
    })());
    return;
  }

  // The build that just replaced this one did not start. Put back the one
  // that did, and remember not to promote this deploy again.
  if (message?.type === "cowboy.rollback-shell") {
    const port = event.ports?.[0];
    event.waitUntil(rollbackShell().then((result) => port?.postMessage(result)));
    return;
  }
  if (message?.type === "cowboy.active-session" && event.source?.id) {
    if (SAFE_SESSION_ID.test(message.sessionId ?? "")) ACTIVE_SESSIONS.set(event.source.id, message.sessionId);
    else ACTIVE_SESSIONS.delete(event.source.id);
    return;
  }
  if (
    !message ||
    message.type !== "cowboy.session-notification" ||
    !validNotificationMessage(message)
  ) return;
  event.waitUntil(showSessionNotification(message));
});

self.addEventListener("push", (event) => {
  let message;
  try {
    message = event.data?.json();
  } catch {
    return;
  }
  if (!validNotificationMessage(message)) return;
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    const alreadyVisible = windows.some((client) =>
      client.visibilityState === "visible" && ACTIVE_SESSIONS.get(client.id) === message.sessionId
    );
    if (!alreadyVisible) await showSessionNotification(message);
  })());
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const data = event.notification.data;
  if (data?.url === "/") {
    const target = new URL("/", self.location.origin).href;
    event.waitUntil((async () => {
      const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      const client = windows.find((candidate) => new URL(candidate.url).origin === self.location.origin);
      if (client) {
        await client.focus();
        return;
      }
      await self.clients.openWindow(target);
    })());
    return;
  }
  const sessionId = data && SAFE_SESSION_ID.test(data.sessionId ?? "")
    ? data.sessionId
    : null;
  if (!sessionId) return;
  const target = new URL(`/?session=${encodeURIComponent(sessionId)}`, self.location.origin).href;
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    const client = windows.find((candidate) => new URL(candidate.url).origin === self.location.origin);
    if (client) {
      await client.navigate(target);
      await client.focus();
      return;
    }
    await self.clients.openWindow(target);
  })());
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  // Product identity is session-specific and must never be served from a SW
  // cache (login/logout, activate 404/501, and HISTORY_CACHE isolation).
  if (url.pathname.startsWith("/api/auth/") || url.pathname.startsWith("/api/admin/")) {
    event.respondWith(fetch(request));
    return;
  }

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

  // Navigations: the cached shell answers at once and the deployed one is
  // fetched behind it (see the header). Only a device with no shell at all, or
  // an explicit request for the deployed build, waits for the network.
  if (request.mode === "navigate") {
    // The system-Safari Passkey ceremony is a short-lived security surface,
    // not the application shell. Never cache it as the offline root document.
    if (url.pathname === "/passkey.html") {
      event.respondWith(fetch(request));
      return;
    }
    // The admin console is a separate Vite entry. Never pin /admin, /admin/*,
    // or /admin.html as the PWA shell and never serve the chat shell when
    // /admin is offline. A successful admin navigation must not write SHELL_CACHE["/"].
    if (
      url.pathname === "/admin" ||
      url.pathname.startsWith("/admin/") ||
      url.pathname === "/admin.html"
    ) {
      event.respondWith(fetch(request));
      return;
    }
    event.respondWith((async () => {
      const wantsDeployed = SHELL_NETWORK_PARAMS.some((name) => url.searchParams.has(name));
      if (!wantsDeployed) {
        const cached = await cachedShell();
        if (cached) {
          event.waitUntil(refreshShell());
          return cached;
        }
      }
      try {
        const resp = await fetch(request);
        if (resp.ok && resp.type === "basic") {
          const copy = resp.clone();
          event.waitUntil(caches.open(SHELL_CACHE).then((c) => c.put("/", copy)));
        }
        return resp;
      } catch {
        return (await cachedShell()) ?? Response.error();
      }
    })());
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
