/** Shared CDP plumbing for fixtures run in an already running Chrome reached
 * over a loopback DevTools endpoint (hawk `chrome-debug` :9222 or the
 * SSH-forwarded macbook-air Chrome :9223).
 *
 * Isolation: a disposable browser context (own storage, no cookies or
 * credentials) and a fictitious origin whose every request is answered from
 * this process through CDP `Fetch` interception. Nothing is exposed on the
 * network and the page never reaches a product endpoint.
 */

export const FIXTURE_ORIGIN = "https://cowboy-conformance.invalid";

// deno-lint-ignore no-explicit-any
export type CdpParams = any;

export interface FixturePage {
  browser: string;
  digest: string;
  send: (method: string, params?: object) => Promise<CdpParams>;
  /** Resolves with the JSON body POSTed to `/report`. */
  report: Promise<CdpParams>;
  evaluate: <T>(expression: string) => Promise<T>;
  close: () => Promise<void>;
}

function encode(bytes: Uint8Array): string {
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  }
  return btoa(binary);
}

export async function openFixturePage(
  endpoint: string,
  suite: string,
  html: (token: string) => string,
): Promise<FixturePage> {
  if (!endpoint.startsWith("http://127.0.0.1:")) {
    throw new Error("the DevTools endpoint must be loopback");
  }
  const temporary = await Deno.makeTempDir({ prefix: "cowboy-cdp-fixture-" });
  let script: Uint8Array<ArrayBuffer>;
  try {
    const built = await new Deno.Command("node", {
      args: ["tools/idb-browser-bundle.mjs", temporary, suite],
      stdout: "null",
      stderr: "inherit",
    }).output();
    if (!built.success) throw new Error("browser fixture bundle failed");
    script = await Deno.readFile(`${temporary}/fixture.js`);
  } finally {
    await Deno.remove(temporary, { recursive: true });
  }
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", script)),
  ).map((byte) => byte.toString(16).padStart(2, "0")).join("");
  const token = crypto.randomUUID();
  const page = new TextEncoder().encode(html(token));

  const version = await (await fetch(`${endpoint}/json/version`)).json() as {
    Browser: string;
    webSocketDebuggerUrl: string;
  };
  const socket = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  let nextId = 0;
  const pending = new Map<number, PromiseWithResolvers<CdpParams>>();
  const listeners:
    ((method: string, params: CdpParams, session?: string) => void)[] = [];
  socket.onmessage = (message) => {
    const data = JSON.parse(message.data as string);
    if (data.id !== undefined) {
      const waiter = pending.get(data.id);
      pending.delete(data.id);
      if (data.error) waiter?.reject(new Error(JSON.stringify(data.error)));
      else waiter?.resolve(data.result);
      return;
    }
    for (const listener of listeners) {
      listener(data.method, data.params, data.sessionId);
    }
  };
  const raw = (method: string, params: object = {}, sessionId?: string) => {
    const id = ++nextId;
    const waiter = Promise.withResolvers<CdpParams>();
    pending.set(id, waiter);
    socket.send(
      JSON.stringify({
        id,
        method,
        params,
        ...(sessionId ? { sessionId } : {}),
      }),
    );
    return waiter.promise;
  };
  const { browserContextId } = await raw("Target.createBrowserContext", {
    disposeOnDetach: true,
  });
  const close = async (): Promise<void> => {
    await raw("Target.disposeBrowserContext", { browserContextId }).catch(
      () => {},
    );
    socket.close();
  };
  try {
    const { targetId } = await raw("Target.createTarget", {
      url: "about:blank",
      browserContextId,
      newWindow: true,
      background: true,
    });
    const { sessionId } = await raw("Target.attachToTarget", {
      targetId,
      flatten: true,
    });
    const send = (method: string, params: object = {}) =>
      raw(method, params, sessionId);
    const report = Promise.withResolvers<CdpParams>();
    listeners.push((method, params, session) => {
      if (session !== sessionId || method !== "Fetch.requestPaused") return;
      const url = new URL(params.request.url);
      const reply = (status: number, type: string, body: Uint8Array) =>
        send("Fetch.fulfillRequest", {
          requestId: params.requestId,
          responseCode: status,
          responseHeaders: [{ name: "Content-Type", value: type }],
          body: encode(body),
        }).catch(() => {});
      if (url.origin !== FIXTURE_ORIGIN) {
        void send("Fetch.failRequest", {
          requestId: params.requestId,
          errorReason: "BlockedByClient",
        }).catch(() => {});
      } else if (url.pathname === "/fixture.js") {
        void reply(200, "text/javascript", script);
      } else if (url.pathname === `/report/${token}`) {
        report.resolve(JSON.parse(params.request.postData ?? "null"));
        void reply(200, "text/plain", new TextEncoder().encode("ok"));
      } else if (url.pathname === `/${token}`) {
        void reply(200, "text/html", page);
      } else {
        void reply(404, "text/plain", new TextEncoder().encode("not found"));
      }
    });
    await send("Fetch.enable", { patterns: [{ urlPattern: "*" }] });
    // A background window must still behave like the focused Desktop app.
    await send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await send("Emulation.setDeviceMetricsOverride", {
      width: 1366,
      height: 768,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Page.navigate", { url: `${FIXTURE_ORIGIN}/${token}` });
    const evaluate = async <T>(expression: string): Promise<T> => {
      const result = await send("Runtime.evaluate", {
        expression,
        awaitPromise: true,
        returnByValue: true,
      });
      if (result.exceptionDetails) {
        throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 600));
      }
      return result.result.value as T;
    };
    return {
      browser: version.Browser,
      digest,
      send,
      report: report.promise,
      evaluate,
      close,
    };
  } catch (error) {
    await close();
    throw error;
  }
}
