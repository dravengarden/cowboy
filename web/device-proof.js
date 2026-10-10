// Shared by the page and its service worker, including protected image loads.
// The private CryptoKey is non-exportable and never leaves this origin's IDB.
(() => {
  if (globalThis.CowboyDeviceProof) return;
  const rawFetch = globalThis.fetch.bind(globalThis);
  const CHALLENGE = "/api/auth/browser/challenge";
  let identityPromise;
  let challengePromise;
  let installed = false;

  function base64(bytes) {
    return btoa(String.fromCharCode(...bytes)).replaceAll("+", "-")
      .replaceAll("/", "_").replaceAll("=", "");
  }

  function usable(keys) {
    return keys?.privateKey instanceof CryptoKey && keys.publicKey instanceof CryptoKey;
  }

  async function identity() {
    if (identityPromise) return identityPromise;
    identityPromise = (async () => {
      const db = await new Promise((resolve, reject) => {
        const opening = indexedDB.open("cowboy-device-identity", 1);
        opening.onupgradeneeded = () => opening.result.createObjectStore("keys");
        opening.onsuccess = () => resolve(opening.result);
        opening.onerror = () => reject(opening.error);
        opening.onblocked = () => reject(new Error("Device identity storage is blocked"));
      });
      db.onversionchange = () => db.close();
      try {
        const saved = await new Promise((resolve, reject) => {
          const tx = db.transaction("keys", "readonly");
          const read = tx.objectStore("keys").get("identity");
          tx.oncomplete = () => resolve(read.result);
          tx.onabort = () => reject(tx.error);
        });
        if (usable(saved)) return saved;
        const candidate = await crypto.subtle.generateKey(
          { name: "ECDSA", namedCurve: "P-256" }, false, ["sign", "verify"],
        );
        // Generation is asynchronous. Recheck under the writer transaction so
        // a concurrent tab/worker cannot replace a just-registered key.
        return await new Promise((resolve, reject) => {
          const tx = db.transaction("keys", "readwrite");
          const store = tx.objectStore("keys");
          const read = store.get("identity");
          let selected;
          read.onsuccess = () => {
            if (usable(read.result)) {
              selected = read.result;
              return;
            }
            // WebKit reads a stored CryptoKey it can no longer unwrap as
            // undefined while the record still exists. add() then failed with
            // ConstraintError on every load, so the device could never sign
            // again. Replace the unreadable identity; this forces a new sign-in.
            selected = candidate;
            store.put(candidate, "identity");
          };
          tx.oncomplete = () => resolve(selected);
          tx.onabort = () => reject(tx.error);
        });
      } finally {
        db.close();
      }
    })().catch((error) => { identityPromise = undefined; throw error; });
    return identityPromise;
  }

  async function challenge() {
    if (!challengePromise) {
      challengePromise = (async () => {
        const response = await rawFetch(CHALLENGE, {
          credentials: "omit", cache: "no-store", redirect: "error",
        });
        if (!response.ok) throw new Error("Secure device authentication is unavailable");
        const body = await response.json();
        if (typeof body.epoch !== "string" || !Number.isSafeInteger(body.server_time_ms)) {
          throw new Error("Invalid device authentication challenge");
        }
        return { epoch: body.epoch, offset: body.server_time_ms - Date.now() };
      })().catch((error) => { challengePromise = undefined; throw error; });
    }
    return challengePromise;
  }

  // Name the failed step. A proof failure happens before dispatch, so the
  // server never sees it; the sign-in bridge shows this message instead.
  async function step(name, operation) {
    try {
      return await operation;
    } catch (error) {
      const detail = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
      throw new Error(`${name} failed (${detail})`, { cause: error });
    }
  }

  async function proof(url, method = "GET") {
    const target = new URL(url, globalThis.location.origin);
    if (target.protocol === "wss:") target.protocol = "https:";
    if (target.origin !== globalThis.location.origin || target.protocol !== "https:") {
      throw new Error("Device proofs require the Cowboy HTTPS origin");
    }
    const [keys, boot] = await Promise.all([
      step("Device identity", identity()), step("Device challenge", challenge()),
    ]);
    const key = base64(new Uint8Array(
      await step("Device key export", crypto.subtle.exportKey("raw", keys.publicKey)),
    ));
    const value = {
      key, epoch: boot.epoch, origin: target.origin,
      time: Date.now() + boot.offset,
      nonce: base64(crypto.getRandomValues(new Uint8Array(32))),
    };
    const message = [
      "cowboy-browser-proof-v1", value.epoch, value.origin, key,
      method.toUpperCase(), target.pathname + target.search, value.time, value.nonce,
    ].join("\n");
    value.signature = base64(new Uint8Array(await step("Device signature", crypto.subtle.sign(
      { name: "ECDSA", hash: "SHA-256" }, keys.privateKey, new TextEncoder().encode(message),
    ))));
    return base64(new TextEncoder().encode(JSON.stringify(value)));
  }

  async function signedFetch(input, init) {
    const request = new Request(input instanceof Request ? input : new URL(input, location.href), init);
    const url = new URL(request.url);
    if (request.mode === "navigate" || url.origin !== location.origin || !url.pathname.startsWith("/api/") || url.pathname === CHALLENGE) {
      return rawFetch(request);
    }
    // Retain a copy before sending, including streaming/FormData bodies. Retry
    // only the explicit pre-dispatch proof rejection, never a business failure.
    const retained = request.clone();
    async function send(source, refresh) {
      const headers = new Headers(source.headers);
      if (refresh || !headers.has("x-cowboy-browser-proof")) {
        headers.set("x-cowboy-browser-proof", await proof(source.url, source.method));
      }
      return rawFetch(new Request(source, { headers, mode: "same-origin", redirect: "error" }));
    }
    const response = await send(request, false);
    if (response.status === 401 && response.headers.get("x-cowboy-device-proof") === "required") {
      challengePromise = undefined;
      return send(retained, true);
    }
    return response;
  }

  globalThis.CowboyDeviceProof = {
    proof,
    resetChallenge() { challengePromise = undefined; },
    install() {
      if (!installed) { globalThis.fetch = signedFetch; installed = true; }
    },
  };
})();
