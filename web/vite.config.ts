import { realpathSync } from "node:fs";
import { dirname } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, proxy the WebSocket + health endpoints to a locally running daemon
// (`cowboy serve`). Override the target with COWBOY_DEV_BACKEND.
const devBackend = process.env.COWBOY_DEV_BACKEND ?? "http://127.0.0.1:3333";

// The shared app-shell SDK lives in the sibling atlantis project and is
// referenced (not vendored): `web/src/_shell` is a symlink to
// `projects/atlantis/main/components` for local dev (the build stages real
// copies via the flake). Resolve the symlink target so the dev server is
// allowed to serve it. Best-effort: absent symlink (e.g. fresh checkout) → skip.
let shellRealRoot: string | undefined;
try {
  shellRealRoot = dirname(realpathSync("src/_shell"));
} catch {
  shellRealRoot = undefined;
}

export default defineConfig({
  build: {
    // Built SPA is embedded into the cowboy binary via rust-embed (folder =
    // "web/dist"); see src/server.rs.
    outDir: "dist",
    emptyOutDir: true,
    // Mobile-first: keep initial entry chunk small (<250 KB pre-gzip) by
    // splitting heavy libs into their own chunks. The browser parallelises
    // chunk fetches, and the syntax-highlighter chunk only loads once a
    // message bubble renders (Markdown is React.lazy'd).
    rollupOptions: {
      output: {
        manualChunks(id: string): string | undefined {
          // Only split out what's behind a React.lazy boundary. Splitting
          // react / emotion / mui across chunks creates a Rollup cycle
          // ("Cannot access 'X' before initialization" at runtime — these
          // libs use top-level side effects + cross-imports). They go in
          // the main entry chunk together; lazy chunks load in parallel
          // via modulepreload anyway.
          if (id.includes("node_modules")) {
            // CodeMirror (editor core + autocomplete + uiw wrapper + lezer +
            // its small deps). Split from the entry so the ~400 KB editor
            // fetches in parallel with — and caches independently of — the app
            // shell. No react/emotion cross-import cycle here, so it's safe to
            // hoist out (unlike react/mui). Vim is already its own lazy chunk
            // (desktop-only dynamic import), so it's not pulled in here.
            if (
              id.includes("@codemirror") ||
              id.includes("@uiw/react-codemirror") ||
              id.includes("@lezer") ||
              id.includes("/crelt/") ||
              id.includes("/style-mod/") ||
              id.includes("/w3c-keyname/")
            ) {
              return "editor";
            }
            if (
              id.includes("react-syntax-highlighter") ||
              id.includes("refractor") ||
              id.includes("/highlight.js/") ||
              id.includes("prismjs")
            ) {
              return "highlighter";
            }
            if (
              id.includes("react-markdown") ||
              id.includes("remark") ||
              id.includes("rehype") ||
              id.includes("micromark") ||
              id.includes("mdast") ||
              id.includes("hast") ||
              id.includes("unified")
            ) {
              return "markdown";
            }
          }
          return undefined;
        },
      },
    },
    chunkSizeWarningLimit: 800,
  },
  // The app-shell SDK is imported through the `_shell` symlink into the sibling
  // atlantis project. Without deduping, vite/rollup resolves those files'
  // `react` / `@mui` / `@emotion` imports relative to the symlink TARGET (the
  // atlantis tree, which has no resolvable copy) and the build fails. Dedupe
  // forces the shared singletons to resolve from this app's own node_modules —
  // which is also what we want at runtime (one React, one MUI, one Emotion).
  resolve: {
    dedupe: [
      "react",
      "react-dom",
      "@mui/material",
      "@mui/icons-material",
      "@emotion/react",
      "@emotion/styled",
    ],
  },
  plugins: [react()],
  server: {
    // Allow the dev server to serve the symlinked atlantis shell source (it
    // lives outside this project root). Only relevant to `dev-web`.
    fs: { allow: [".", "..", ...(shellRealRoot ? [shellRealRoot] : [])] },
    proxy: {
      "/ws": { target: devBackend, ws: true },
      "/healthz": devBackend,
      "/api": devBackend,
    },
  },
});
