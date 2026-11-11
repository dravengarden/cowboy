// Builds the full-DOM app runtime and shell (docs/widget-apps-dom.md §3): the
// runtime is one ES module string (React, ReactDOM, Three, Sucrase) the host
// sends over the verified port, and app-shell.html is the one document the
// Controller serves at /app-shell.html with its own headers. Plain JavaScript
// so the conformance bundler (tools/idb-browser-bundle.mjs) can load it.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { build } from "vite";

const VIRTUAL = "virtual:cowboy-dom-runtime";
const RESOLVED = `\0${VIRTUAL}`;
const dom = (file) => fileURLToPath(new URL(`./src/widgets/dom/${file}`, import.meta.url));

/** The shell document; its one inline script is hashed by the Controller. */
export function appShellHtml() {
  const boot = readFileSync(dom("shellBoot.js"), "utf8");
  if (boot.includes("</script")) throw new Error("the shell script cannot contain </script");
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><style>html,body{margin:0}</style></head><body><div id="root"></div><script>${boot}</script></body></html>\n`;
}

async function bundleRuntime() {
  const result = await build({
    configFile: false,
    logLevel: "silent",
    define: { "process.env.NODE_ENV": '"production"' },
    build: {
      write: false,
      minify: true,
      emptyOutDir: false,
      lib: { entry: dom("runtimeEntry.js"), formats: ["es"], fileName: "runtime" },
      rolldownOptions: { output: { codeSplitting: false } },
    },
  });
  const outputs = Array.isArray(result) ? result : [result];
  const chunk = outputs[0]?.output.find((item) => item.type === "chunk");
  if (!chunk) throw new Error("the DOM app runtime bundle produced no code");
  return chunk.code;
}

/** @returns {import("vite").Plugin} */
export function cowboyDomRuntime() {
  /** @type {Promise<string> | null} */
  let runtime = null;
  return {
    name: "cowboy-dom-runtime",
    resolveId: (source) => (source === VIRTUAL ? RESOLVED : null),
    async load(id) {
      if (id !== RESOLVED) return null;
      runtime ??= bundleRuntime();
      return `export const runtimeSource = ${JSON.stringify(await runtime)};\nexport const shellHtml = ${JSON.stringify(appShellHtml())};\n`;
    },
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "app-shell.html", source: appShellHtml() });
    },
  };
}
