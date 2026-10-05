import { EDITOR_PLUGIN_WORKER_PRELUDE } from "./workerPrelude";

/** One plugin's message channel. The runtime never sees the realm behind it. */
export interface EditorPluginTransport {
  post(message: unknown): void;
  /** Delivers worker messages; a sandbox failure arrives as `{ t: "crash" }`. */
  listen(listener: (message: unknown) => void): void;
  terminate(): void;
}

// The sandbox document only boots the Worker. `allow-scripts` without
// `allow-same-origin` gives it an opaque origin: no App cookies, storage, DOM or
// same-origin API access. Its CSP removes network, frames, forms and images;
// the Worker inherits that policy because it is created from a blob of this
// document. The Worker keeps a busy plugin off the App's main thread, and
// removing the iframe terminates it.
const SANDBOX_DOCUMENT = `<!doctype html><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' blob:; worker-src blob:">
<script>
addEventListener("message", (event) => {
  if (event.source !== parent || !event.data || event.data.t !== "cowboy-boot" || event.ports.length !== 1) return;
  try {
    const url = URL.createObjectURL(new Blob([event.data.code], { type: "text/javascript" }));
    const worker = new Worker(url);
    worker.addEventListener("error", (e) => {
      e.preventDefault();
      parent.postMessage({ t: "cowboy-sandbox-error", error: String(e.message || "Plugin failed to start") }, "*");
    });
    worker.postMessage(null, [event.ports[0]]);
  } catch (error) {
    parent.postMessage({ t: "cowboy-sandbox-error", error: String(error) }, "*");
  }
}, { once: true });
parent.postMessage({ t: "cowboy-sandbox-ready" }, "*");
</script>`;

export function createSandboxTransport(main: string): EditorPluginTransport {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("aria-hidden", "true");
  frame.tabIndex = -1;
  frame.dataset.cowboyEditorPluginSandbox = "true";
  frame.style.cssText =
    "position:fixed;width:0;height:0;border:0;opacity:0;pointer-events:none;inset:auto";
  frame.srcdoc = SANDBOX_DOCUMENT;
  const channel = new MessageChannel();
  const listeners: ((message: unknown) => void)[] = [];
  const queued: unknown[] = [];
  let closed = false;
  const deliver = (message: unknown): void => {
    if (closed) return;
    if (listeners.length === 0) queued.push(message);
    for (const listener of listeners) listener(message);
  };
  const onWindowMessage = (event: MessageEvent): void => {
    if (event.source !== frame.contentWindow || closed) return;
    const data = event.data as { t?: string; error?: string } | null;
    if (data?.t === "cowboy-sandbox-ready") {
      frame.contentWindow?.postMessage(
        {
          t: "cowboy-boot",
          code: `${EDITOR_PLUGIN_WORKER_PRELUDE}\n;(() => {\n${main}\n})();\n`,
        },
        "*",
        [channel.port2],
      );
    } else if (data?.t === "cowboy-sandbox-error") {
      deliver({ t: "crash", error: String(data.error ?? "Sandbox failed") });
    }
  };
  globalThis.addEventListener("message", onWindowMessage);
  channel.port1.onmessage = (event) => deliver(event.data);
  document.body.append(frame);
  return {
    post: (message) => {
      if (!closed) channel.port1.postMessage(message);
    },
    listen: (listener) => {
      listeners.push(listener);
      for (const message of queued.splice(0)) listener(message);
    },
    terminate: () => {
      if (closed) return;
      closed = true;
      globalThis.removeEventListener("message", onWindowMessage);
      channel.port1.close();
      frame.remove();
    },
  };
}
