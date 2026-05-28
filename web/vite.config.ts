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
  },
  plugins: [react()],
  server: {
    proxy: {
      "/ws": { target: devBackend, ws: true },
      "/healthz": devBackend,
    },
  },
});
