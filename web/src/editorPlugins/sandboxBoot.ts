/** The srcdoc sandbox's inline script. The frame also inherits the App document's
 * CSP, which allows no inline script except by hash, so the build copies this
 * text byte for byte into `index.html` as a data block and the server hashes
 * every inline block of the document it serves. Keep this file free of imports:
 * `vite.config.ts` reads it as well. */
export const EDITOR_PLUGIN_SANDBOX_BOOT = `
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
`;

export const EDITOR_PLUGIN_SANDBOX_BOOT_ID = "cowboy-editor-sandbox-boot";
