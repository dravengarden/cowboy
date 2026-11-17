/** Signed MCP Apps views (docs/widget-apps-mcp.md §9): the production frame
 *  host (`startMcpApp`) and bridge run the conformance view the runner serves
 *  at `/mcp-app/<token>.html` with the Controller's real headers. The
 *  Controller's lease and call endpoints are substituted here, so this page
 *  records what the host was asked and did; egress is observed outside the
 *  page (tools/mcp-apps-conformance.ts). */
import { setMcpAppTransport, startMcpApp } from "./widgets/dom/mcpAppHost";

type Conformance = {
  log: string[];
  start: (mode: "main" | "flood" | "navigate", token: string) => void;
};

const state: Conformance = {
  log: [],
  start(mode, token) {
    const log = (entry: string): void => {
      state.log.push(`${mode}:${entry}`);
    };
    setMcpAppTransport(async (method, url, body) => {
      const request = (body ?? {}) as Record<string, unknown>;
      if (url.endsWith("/renew")) return { status: 200, body: { lease_ms: 30_000 } };
      if (method === "DELETE") return { status: 204, body: {} };
      if (url.endsWith("/open-link"))
        return { status: 200, body: { state: "ok", url: request["url"] } };
      if (url.endsWith("/call")) {
        const name = String(request["name"]);
        log(`call:${name}:${request["confirmed"] ? "confirmed" : "plain"}`);
        if (name === "fixture_write" && !request["confirmed"]) {
          return {
            status: 200,
            body: { state: "confirm", digest: "d".repeat(64), arguments: request["arguments"] },
          };
        }
        return {
          status: 200,
          body: {
            state: "ok",
            response: {
              jsonrpc: "2.0",
              id: "view-1",
              result: { content: [{ type: "text", text: `${name} ok` }] },
            },
          },
        };
      }
      return { status: 404, body: { error: "unexpected" } };
    });
    const container = document.createElement("div");
    container.style.width = "480px";
    document.body.append(container);
    startMcpApp(
      container,
      {
        session: "s",
        frame: {
          url: `/mcp-app/${token}.html`,
          lease: `lease-${mode}`,
          leaseMs: 30_000,
          capabilities: {
            call_tools: ["fixture_status", "fixture_write"],
            message: true,
            open_links: true,
          },
        },
        hostContext: { theme: "light", locale: "en", platform: "web" },
        version: "conformance",
        toolInput: { mode },
        toolResult: { content: [{ type: "text", text: "status green" }] },
      },
      {
        onHeight: (height) => log(`height:${height}`),
        confirmCall: async (call) => {
          log(`confirm:${call.tool}:${JSON.stringify(call.arguments)}`);
          return true;
        },
        message: async (text) => {
          log(`message:${text}`);
          return true;
        },
        openLink: (url) => log(`link:${url}`),
        onStopped: (reason) => log(`stopped:${reason}`),
      },
    );
  },
};

export function installMcpAppsConformance(): void {
  (globalThis as typeof globalThis & { __mcpApps?: Conformance }).__mcpApps = state;
}
