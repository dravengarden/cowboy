// Builds the widget app runtime (docs/widget-apps.md §3) as two self-contained
// script strings: the VM bundle (React, reconciler, Sucrase, @cowboy/ui) that
// runs inside QuickJS, and the trusted Worker that hosts it. The host passes
// both into the sandbox as message data; neither is fetched by the sandbox.
// Plain JavaScript so the conformance bundler (tools/idb-browser-bundle.mjs)
// can load it without a TypeScript step.
import { fileURLToPath } from "node:url";
import { build } from "vite";

const VIRTUAL = "virtual:cowboy-app-runtime";
const RESOLVED = `\0${VIRTUAL}`;
const runtime = (file) =>
  fileURLToPath(new URL(`./src/widgets/apps/runtime/${file}`, import.meta.url));

/** @param {string} entry @returns {Promise<string>} */
async function bundle(entry) {
  const result = await build({
    configFile: false,
    logLevel: "silent",
    define: { "process.env.NODE_ENV": '"production"' },
    // The QuickJS wasm arrives as bytes from the host (`wasmBinary`). Library
    // mode would inline the glue's `new URL(…wasm)` as half a megabyte of
    // base64, so that reference is replaced by an unusable name.
    plugins: [
      {
        name: "cowboy-quickjs-wasm-by-host",
        transform: (code, id) =>
          id.includes("quickjs-wasmfile")
            ? code.replaceAll('new URL("emscripten-module.wasm",import.meta.url)', '"about:blank"')
            : null,
      },
    ],
    build: {
      write: false,
      minify: true,
      emptyOutDir: false,
      lib: { entry, formats: ["iife"], name: "CowboyAppRuntime" },
      rolldownOptions: { output: { codeSplitting: false } },
    },
  });
  const outputs = Array.isArray(result) ? result : [result];
  const chunk = outputs[0]?.output.find((item) => item.type === "chunk");
  if (!chunk) throw new Error(`app runtime bundle ${entry} produced no code`);
  return chunk.code;
}

/** @returns {import("vite").Plugin} */
export function cowboyAppRuntime() {
  /** @type {Promise<{ vm: string; worker: string }> | null} */
  let sources = null;
  return {
    name: "cowboy-app-runtime",
    resolveId: (source) => (source === VIRTUAL ? RESOLVED : null),
    async load(id) {
      if (id !== RESOLVED) return null;
      sources ??= Promise.all([
        bundle(runtime("vmEntry.js")),
        bundle(runtime("workerEntry.js")),
      ]).then(([vm, worker]) => ({ vm, worker }));
      const { vm, worker } = await sources;
      return `export const vmSource = ${JSON.stringify(vm)};\nexport const workerSource = ${JSON.stringify(worker)};\n`;
    },
  };
}
