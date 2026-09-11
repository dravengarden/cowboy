// Use the same lock-pinned, checkout-local Vite builder as the product Web.
// No bundler download is permitted in the conformance network namespace.
import { build } from "../web/node_modules/vite/dist/node/index.js";

const outDir = process.argv[2];
if (!outDir?.startsWith("/")) {
  throw new Error("expected an absolute fixture output directory");
}
await build({
  configFile: false,
  envDir: false,
  publicDir: false,
  resolve: {
    alias: {
      "@cowboy/state-store/scope":
        new URL("../components/state-store/owned-scope.ts", import.meta.url)
          .pathname,
    },
  },
  build: {
    outDir,
    emptyOutDir: false,
    minify: false,
    lib: {
      entry: new URL("../web/src/idbBrowserConformance.ts", import.meta.url)
        .pathname,
      formats: ["es"],
      fileName: () => "fixture.js",
    },
  },
});
