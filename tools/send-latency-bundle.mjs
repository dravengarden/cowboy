// Build before and after from the owning checkout, retaining both artifacts.
import { build } from "../web/node_modules/vite/dist/node/index.js";
import { readFileSync, writeFileSync } from "node:fs";
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
      entry: new URL(process.argv[3] === "--device-ui"
        ? "../web/src/deviceUiFixture.tsx"
        : process.argv[3] === "--metadata"
        ? "../web/src/metadataLatencyBrowserFixture.tsx"
        : process.argv[3] === "--local"
        ? "../web/src/localLatencyBrowserFixture.tsx"
        : "../web/src/sendLatencyBrowserFixture.ts", import.meta.url).pathname,
      formats: ["es"],
      fileName: () => "fixture.js",
    },
  },
});
if (process.argv[3] === "--device-ui") {
  const source = readFileSync(new URL("../web/index.html", import.meta.url), "utf8");
  const styles = source.match(/<style>([\s\S]*?)<\/style>/)?.[1] ?? "";
  writeFileSync(`${outDir}/index.html`, `<!doctype html><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover, interactive-widget=resizes-content">
<style>${styles}</style><link rel="stylesheet" href="cowboy-web.css"><div id="root"></div>
<script type="module">import {run} from './fixture.js'; run().catch(e => document.body.textContent=String(e));</script>`);
}
