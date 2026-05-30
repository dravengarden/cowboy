import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

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
      output: {
        manualChunks(id: string): string | undefined {
          // Only split out what's behind a React.lazy boundary. Splitting
          // react / emotion / mui across chunks creates a Rollup cycle
          // ("Cannot access 'X' before initialization" at runtime — these
          // libs use top-level side effects + cross-imports). They go in
          // the main entry chunk together; lazy chunks load in parallel
          // via modulepreload anyway.
          if (id.includes("node_modules")) {
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
  plugins: [react()],
  server: {
    proxy: {
      "/ws": { target: devBackend, ws: true },
      "/healthz": devBackend,
      "/api": devBackend,
    },
  },
});
