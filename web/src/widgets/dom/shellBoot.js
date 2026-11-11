// The full-DOM app shell's only script (docs/widget-apps-dom.md §3, §4). It is
// inlined into /app-shell.html and authorized there by its hash. It loads
// nothing over the network: the runtime, the app's code, props and images
// arrive over one verified MessagePort, and every module runs from a blob:
// URL this document creates. Plain JavaScript; nothing here is agent-written.
(() => {
  // The shell shares Cowboy's URL origin; its document origin is opaque.
  const PARENT = location.origin;
  let started = false;
  const blob = (body, type = "text/javascript") => URL.createObjectURL(new Blob([body], { type }));

  addEventListener("message", (event) => {
    if (started || event.source !== parent || event.origin !== PARENT) return;
    const data = event.data;
    if (!data || data.t !== "cowboy-dom-port" || event.ports.length !== 1) return;
    if (typeof data.nonce !== "string" || typeof data.generation !== "string") return;
    started = true;
    const port = event.ports[0];
    port.onmessage = (message) => onPort(port, message.data);
    port.postMessage({ t: "nonce", nonce: data.nonce, generation: data.generation });
  });
  parent.postMessage({ t: "cowboy-dom-ready" }, PARENT);

  let render = null;

  function onPort(port, message) {
    if (!message || typeof message !== "object") return;
    if (message.t === "ping") return void port.postMessage({ t: "pong", n: message.n });
    if (message.t === "props" && render) return void render(message.props);
    if (message.t === "boot" && !render) void boot(port, message);
  }

  async function boot(port, message) {
    try {
      const runtimeUrl = blob(message.runtime);
      const runtime = await import(runtimeUrl);
      const images = {};
      for (const [key, bytes] of Object.entries(message.images || {})) {
        images[key] = blob(bytes, "image/png");
      }
      globalThis.__cowboyDom = {
        send: (text) => port.postMessage({ t: "send", text: String(text).slice(0, 8000) }),
        images: Object.freeze(images),
        context: Object.freeze({ ...(message.context || {}) }),
      };
      // Imports resolve through require(), which Sucrase's imports transform
      // emits for every import form; no source text is rewritten.
      const modules = {
        react: runtime.React,
        "react/jsx-runtime": runtime.JsxRuntime,
        "react-dom/client": runtime.ReactDOM,
        three: runtime.THREE,
        "@cowboy/dom": Object.freeze({
          useSend: () => globalThis.__cowboyDom.send,
          images: globalThis.__cowboyDom.images,
          useHostContext: () => globalThis.__cowboyDom.context,
        }),
      };
      globalThis.__cowboyDomRequire = (name) => {
        if (!Object.hasOwn(modules, name)) throw new Error(`import "${name}" is not available in Cowboy apps`);
        return modules[name];
      };
      const compiled = runtime.transform(message.code, {
        transforms: ["typescript", "jsx", "imports"],
        jsxRuntime: "automatic",
        production: true,
      }).code;
      const linked =
        "const require = globalThis.__cowboyDomRequire; const exports = {}; const module = { exports };\n" +
        compiled +
        "\nexport default (module.exports.default ?? exports.default);\n";
      const App = (await import(blob(linked))).default;
      if (typeof App !== "function") throw new Error("the app has no default export component");
      const root = runtime.ReactDOM.createRoot(document.getElementById("root"));
      render = (props) => root.render(runtime.React.createElement(App, props ?? {}));
      render(message.props);
      let last = 0;
      new ResizeObserver(() => {
        const height = Math.ceil(document.documentElement.scrollHeight);
        if (height !== last) port.postMessage({ t: "height", height: (last = height) });
      }).observe(document.documentElement);
      port.postMessage({ t: "booted" });
    } catch (error) {
      port.postMessage({ t: "error", text: String(error && error.message ? error.message : error).slice(0, 300) });
    }
  }
})();
