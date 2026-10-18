import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { App } from "./App";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { managementEntryFixture } from "./providerManagement.fixture";
import {
  loadProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry";
import type { Envelope, SessionMeta } from "./protocol";

export async function runDesktopViewportBrowserConformance(
  keepFixture = false,
): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const originalSocket = globalThis.WebSocket;
  const originalFont = document.documentElement.style.fontSize;
  const session: SessionMeta = {
    id: "desktop-viewport",
    title: "cowboy improve gyjp",
    provider: "example",
    cwd: "/fixture",
    machine_id: "ovh",
    status: "busy",
    origin: "web",
    execution_binding: {
      schema: 1,
      runtime: { machine_id: "ovh", cwd: "/fixture" },
      environment: { protocol: 1, machine_id: "hawk" },
      workspace: { cwd: "/fixture" },
    },
  };
  const entry = managementEntryFixture("example");
  entry.publisher = entry.manifest.publisher;
  entry.manifest.authentication.required = false;
  entry.manifest.authentication.methods = [];
  const timeline: Envelope[] = Array.from({ length: 80 }, (_, i) => ({
    seq: i + 1,
    kind: "update",
    session_id: session.id,
    update: {
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: `Paragraph ${i}: gyjp 中文末行\n\n` },
    },
  }));
  class FixtureSocket {
    static current: FixtureSocket;
    static OPEN = 1;
    static CONNECTING = 0;
    readyState = 1;
    protocol = "cowboy-sync-v1";
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose = null;
    onerror = null;
    constructor() {
      FixtureSocket.current = this;
      setTimeout(() => {
        this.onopen?.();
        this.publish({ type: "sessions", sessions: [session] });
        this.publish({ type: "bootstrap_complete" });
      }, 20);
    }
    publish(message: unknown) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send() {}
    close() {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  bindProductSyncPrincipal("viewport-fixture");
  localStorage.setItem("cowboy:active", session.id);
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input);
    if (!path.startsWith("/api/")) return originalFetch(input, init);
    if (path === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"a".repeat(64)}`,
        user_id: "viewport-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (path === "/api/plugins") {
      return Response.json({
        providers: [entry],
        authentications: [],
        authentication_executors: [],
      });
    }
    if (path.endsWith("/bootstrap")) {
      return Response.json({
        messages: [{
          type: "snapshot",
          session_id: session.id,
          events: timeline,
          reached_start: true,
        }],
      });
    }
    if (path === "/api/usage") {
      return Response.json({ providers: [], observed_at_ms: Date.now() });
    }
    return new Response("Isolated fixture", { status: 404 });
  }) as typeof fetch;
  resetProviderCatalog();
  await loadProviderCatalog();
  const container = document.createElement("div");
  container.style.cssText = "height:100vh;width:100vw;overflow:hidden";
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <SurfaceProvider>
            <DesktopWorkspaceProvider>
              <DesktopCommandProvider>
                <App
                  themeMode="light"
                  onSetThemeMode={() => {}}
                  surface="desktop"
                />
              </DesktopCommandProvider>
            </DesktopWorkspaceProvider>
          </SurfaceProvider>
        </BrowserProductTheme>,
      )
    );
    for (
      let i = 0;
      i < 200 && !container.querySelector("[data-desktop-session-row]");
      i++
    ) await new Promise((r) => setTimeout(r, 25));
    (container.querySelector("[data-desktop-session-row]") as
      | HTMLElement
      | null)?.click();
    for (
      let i = 0;
      i < 200 &&
      !container.querySelector("[data-desktop-transcript-scroller] [data-key]");
      i++
    ) await new Promise((r) => setTimeout(r, 25));
    const scroller = container.querySelector<HTMLElement>(
      "[data-desktop-transcript-scroller]",
    );
    if (!scroller) {
      throw new Error(
        `Desktop transcript did not mount: ${
          container.textContent?.slice(0, 300)
        }`,
      );
    }
    let seq = timeline.length;
    for (const scale of [1, 1.25, 0.8]) {
      document.documentElement.style.fontSize = `${16 * scale}px`;
      await new Promise((r) => setTimeout(r, 100));
      FixtureSocket.current.publish({
        type: "event",
        envelope: {
          ...timeline[0],
          seq: ++seq,
          update: {
            sessionUpdate: "agent_message_chunk",
            content: {
              type: "text",
              text: `Latest ${scale}: gyjp 中文末行\n\n`,
            },
          },
        },
      });
      await new Promise((r) => setTimeout(r, 150));
      const bounds = scroller.getBoundingClientRect();
      const row = scroller.querySelector<HTMLElement>("[data-key]")!;
      if (!row?.textContent?.includes(`Latest ${scale}:`)) {
        throw new Error(`Live transcript did not advance at ${scale}`);
      }
      if (
        !row || row.getBoundingClientRect().bottom > bounds.bottom + 0.5 ||
        bounds.bottom > innerHeight + 0.5
      ) throw new Error(`Transcript bottom clipped at ${scale}`);
      results.push(
        `Latest transcript row stays inside the viewport at ${scale}`,
      );
      const title = container.querySelector<HTMLElement>(
        "[data-desktop-active-session]",
      )!;
      const glyph = title.querySelector("svg")!.getBoundingClientRect();
      const box = title.getBoundingClientRect();
      if (glyph.top < box.top - 0.5 || glyph.bottom > box.bottom + 0.5) {
        throw new Error(`Title glyph clipped at ${scale}`);
      }
      results.push(`Active session glyph fits at ${scale}`);
      const sidebar = container.querySelector<HTMLElement>(
        "[data-desktop-session-row]",
      )!;
      const badge = sidebar.querySelector<HTMLElement>(".MuiChip-root")!;
      const label = badge.querySelector(".MuiChip-label")!;
      const range = document.createRange();
      range.selectNodeContents(label);
      const text = range.getBoundingClientRect();
      const badgeBounds = badge.getBoundingClientRect();
      if (
        text.top < badgeBounds.top - 0.5 ||
        text.bottom > badgeBounds.bottom + 0.5
      ) {
        throw new Error(`Machine badge text clipped at ${scale}`);
      }
      results.push(`Machine badge text fits at ${scale}`);
    }
    return results;
  } finally {
    if (!keepFixture) {
      flushSync(() => root.unmount());
      container.remove();
      document.documentElement.style.fontSize = originalFont;
    }
    globalThis.fetch = originalFetch;
    globalThis.WebSocket = originalSocket;
  }
}
