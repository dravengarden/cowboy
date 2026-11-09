/** The app sandbox frame's inline script (docs/widget-apps.md §4.1–4.2). The
 * frame inherits the App document's CSP, which allows no inline script except
 * by hash, so the build copies this text byte for byte into `index.html` as a
 * data block and the server hashes it. It is trusted code: it never evaluates
 * app source, only starts the runtime Worker from the host's bundle and hands
 * it the port. Keep this file free of imports: `vite.config.ts` reads it. */
export const APP_SANDBOX_BOOT = `
const onBoot = (event) => {
  const d = event.data;
  if (event.source !== parent || !d || d.t !== "cowboy-app-boot" || event.ports.length !== 1) return;
  removeEventListener("message", onBoot);
  try {
    const url = URL.createObjectURL(new Blob([d.worker], { type: "text/javascript" }));
    const worker = new Worker(url);
    worker.addEventListener("error", (e) => {
      e.preventDefault();
      parent.postMessage({ t: "cowboy-app-sandbox-error", error: String(e.message || "The app failed to start") }, "*");
    });
    worker.postMessage(
      { nonce: d.nonce, wasm: d.wasm, vm: d.vm, code: d.code, props: d.props, context: d.context },
      [event.ports[0], d.wasm],
    );
  } catch (error) {
    parent.postMessage({ t: "cowboy-app-sandbox-error", error: String(error) }, "*");
  }
};
addEventListener("message", onBoot);
parent.postMessage({ t: "cowboy-app-sandbox-ready" }, "*");
`;

export const APP_SANDBOX_BOOT_ID = "cowboy-app-sandbox-boot";

/** The frame's own policy, intersected with the inherited App CSP. */
export const APP_SANDBOX_CSP =
  "default-src 'none'; script-src 'unsafe-inline' blob: 'wasm-unsafe-eval'; worker-src blob:";
