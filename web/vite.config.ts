import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const webRoot = dirname(fileURLToPath(import.meta.url));

// In dev, proxy the WebSocket + health endpoints to a locally running daemon
// (`cowboy serve`). Override the target with COWBOY_DEV_BACKEND.
const devBackend = process.env.COWBOY_DEV_BACKEND ?? "http://127.0.0.1:3333";

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
      input: {
        main: resolve(webRoot, "index.html"),
        admin: resolve(webRoot, "admin.html"),
        passkey: resolve(webRoot, "passkey.html"),
      },
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
              // Keep Code Review language packages behind their native dynamic
              // imports. Hoisting every `lang-*`, parser, and legacy mode into
              // `editor` turns opening one source file into a 1.6 MB download.
              // Markdown is the one statically used language (the composer).
              id.includes("@codemirror/lang-markdown") ||
              (
                id.includes("@codemirror") &&
                !id.includes("@codemirror/language-data") &&
                !id.includes("@codemirror/lang-") &&
                !id.includes("@codemirror/legacy-modes")
              ) ||
              id.includes("@uiw/react-codemirror") ||
              id.includes("@lezer/common") ||
              id.includes("@lezer/highlight") ||
              id.includes("@lezer/lr") ||
              id.includes("@lezer/markdown") ||
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
  // App-shell components share the application's React/MUI/Emotion singletons.
  // Keep the dedupe contract explicit so future plugin-facing component entry
  // points cannot introduce a second renderer instance.
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
  plugins: [
    react(),
    {
      name: "cowboy-admin-routes",
      configureServer(server) {
        server.middlewares.use((request, _response, next) => {
          const url = request.url?.split("?")[0] ?? "";
          if (url === "/admin" || url === "/admin/" || url.startsWith("/admin/")) {
            request.url = "/admin.html";
          }
          next();
        });
      },
    },
  ],
  server: {
    fs: { allow: [".", ".."] },
    proxy: {
      "/ws": { target: devBackend, ws: true },
      "/healthz": devBackend,
      "/version": devBackend,
      "/api": devBackend,
    },
  },
});
