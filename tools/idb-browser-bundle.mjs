// Use the same lock-pinned, checkout-local Vite builder as the product Web.
// No bundler download is permitted in the conformance network namespace.
import { build } from "../web/node_modules/vite/dist/node/index.js";

const outDir = process.argv[2];
const suite = process.argv[3] ?? "idb";
if (
  suite !== "idb" && suite !== "idb-outbox" && suite !== "provider-ui" &&
  suite !== "provider-management" && suite !== "plugin-lifecycle" &&
  suite !== "settings-recovery" && suite !== "code-buffers" &&
  suite !== "code-buffer-context" && suite !== "code-buffer-cleanup" &&
  suite !== "code-buffer-sync" && suite !== "review-code" &&
  suite !== "review-document-refresh" && suite !== "review-diff" &&
  suite !== "review-destination" && suite !== "review-recovery"
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
      suite === "provider-ui" || suite === "provider-management" ||
        suite === "plugin-lifecycle" || suite === "settings-recovery" ||
        suite === "code-buffers" || suite === "code-buffer-context" ||
        suite === "code-buffer-cleanup" || suite === "code-buffer-sync" ||
        suite === "review-code" || suite === "review-document-refresh" ||
        suite === "review-diff" || suite === "review-destination" ||
        suite === "review-recovery"
        ? "development"
        : "production",
    ),
  },
  resolve: {
    // Match the product's app-shell peer resolution; Sheet crosses the Web /
    // component boundary and must use this checkout's UI singletons.
    dedupe: [
      "@cowboy/state-store",
      "react",
      "react-dom",
      "@mui/material",
      "@mui/icons-material",
      "@emotion/react",
      "@emotion/styled",
    ],
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
    // CodeMirror's language loaders otherwise extract shared static chunks.
    // Keep the isolated runner's one served/hashed artifact, not an open file server.
    ...(suite === "review-diff" || suite === "review-document-refresh" ||
        suite === "review-destination" || suite === "review-recovery"
      ? { rolldownOptions: { output: { codeSplitting: false } } }
      : {}),
    lib: {
      entry: new URL(
        suite === "provider-ui"
          ? "../web/src/providerUiBrowserConformance.ts"
          : suite === "provider-management"
          ? "../web/src/providerManagementBrowserConformance.ts"
          : suite === "plugin-lifecycle"
          ? "../web/src/pluginLifecycleBrowserConformance.ts"
          : suite === "settings-recovery"
          ? "../web/src/settingsRecoveryBrowserConformance.ts"
          : suite === "code-buffers"
          ? "../web/src/codeBufferBrowserConformance.ts"
          : suite === "code-buffer-context"
          ? "../web/src/codeBufferContextBrowserConformance.ts"
          : suite === "code-buffer-cleanup"
          ? "../web/src/codeBufferCleanupBrowserConformance.tsx"
          : suite === "code-buffer-sync"
          ? "../web/src/codeBufferSynchronizationBrowserConformance.tsx"
          : suite === "review-code"
          ? "../web/src/reviewCodeBrowserConformance.tsx"
          : suite === "review-document-refresh"
          ? "../web/src/reviewDocumentRefreshBrowserConformance.tsx"
          : suite === "review-diff"
          ? "../web/src/reviewDiffBrowserConformance.tsx"
          : suite === "review-destination"
          ? "../web/src/reviewDestinationBrowserConformance.tsx"
          : suite === "review-recovery"
          ? "../web/src/reviewRecoveryBrowserConformance.tsx"
          : suite === "idb-outbox"
          ? "../web/src/idbOutboxBrowserConformance.ts"
          : "../web/src/idbBrowserConformance.ts",
        import.meta.url,
      )
        .pathname,
      formats: ["es"],
      fileName: () => "fixture.js",
    },
  },
});
