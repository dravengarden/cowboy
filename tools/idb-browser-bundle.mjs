// Use the same lock-pinned, checkout-local Vite builder as the product Web.
// No bundler download is permitted in the conformance network namespace.
import { build } from "../web/node_modules/vite/dist/node/index.js";

const outDir = process.argv[2];
const suite = process.argv[3] ?? "idb";
if (
  suite !== "idb" && suite !== "provider-ui" && suite !== "provider-management"
) {
  throw new Error("unknown suite");
}
if (!outDir?.startsWith("/")) {
  throw new Error("expected an absolute fixture output directory");
}
await build({
  root: new URL("../web", import.meta.url).pathname,
  configFile: false,
  envDir: false,
  publicDir: false,
  define: {
    "process.env.NODE_ENV": JSON.stringify(
      suite !== "idb" ? "development" : "production",
    ),
  },
  resolve: {
    dedupe: ["react", "react-dom"],
    alias: {
      "@cowboy/provider-ui":
        new URL("../components/provider-ui/src/index.ts", import.meta.url)
          .pathname,
      "@cowboy/provider-authoring":
        new URL("../components/provider-authoring/index.ts", import.meta.url)
          .pathname,
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
      entry: new URL(
        suite === "provider-ui"
          ? "../web/src/providerUiBrowserConformance.ts"
          : suite === "provider-management"
          ? "../web/src/providerManagementBrowserConformance.ts"
          : "../web/src/idbBrowserConformance.ts",
        import.meta.url,
      )
        .pathname,
      formats: ["es"],
      fileName: () => "fixture.js",
    },
  },
});
