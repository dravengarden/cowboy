// Build before and after from the owning checkout, retaining both artifacts.
import { build } from "../web/node_modules/vite/dist/node/index.js";
const outDir = process.argv[2];
if (!outDir?.startsWith("/")) {
  throw new Error("absolute output directory required");
}
await build({
  root: new URL("../web", import.meta.url).pathname,
  configFile: false,
  envDir: false,
  publicDir: false,
  define: { "process.env.NODE_ENV": '"production"' },
  resolve: {
    dedupe: [
      "react",
      "react-dom",
      "@cowboy/state-store",
      "@mui/material",
      "@mui/icons-material",
      "@emotion/react",
      "@emotion/styled",
    ],
    alias: {
      "@cowboy/provider-ui":
        new URL("../components/provider-ui/src/index.ts", import.meta.url)
          .pathname,
      "@cowboy/state-store/scope":
        new URL("../components/state-store/owned-scope.ts", import.meta.url)
          .pathname,
    },
  },
  build: {
    outDir,
    emptyOutDir: false,
    minify: true,
    lib: {
      entry: new URL("../web/src/sendLatencyBrowserFixture.ts", import.meta.url)
        .pathname,
      formats: ["es"],
      fileName: () => "fixture.js",
    },
  },
});
