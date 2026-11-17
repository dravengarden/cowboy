import { describe, expect, test } from "bun:test";
import {
  INGRESS_BYTES_PER_SECOND,
  INGRESS_MESSAGES_PER_SECOND,
  INGRESS_REQUESTS_PER_SECOND,
  MCP_APPS_PROTOCOL,
  jsonBytes,
  McpBridge,
  type McpBridgeHost,
} from "./mcpAppBridge";
import { mcpAppsEnabledFor } from "./mcpAppsEnabled";

type Sent = Record<string, unknown>;
const err = (message: Sent | undefined): { message?: string; code?: number } =>
  (message?.["error"] ?? {}) as { message?: string; code?: number };

function harness(overrides: Partial<McpBridgeHost> = {}) {
  const sent: Sent[] = [];
  const removed: string[] = [];
  const calls: Array<{ name: string; args: unknown }> = [];
  const sizes: number[] = [];
  let clock = 0;
  const bridge = new McpBridge({
    capabilities: { call_tools: ["suger_status"], message: false, open_links: false },
    hostContext: { theme: "dark", locale: "en", platform: "web" },
    version: "test",
    toolInput: { q: 1 },
    toolResult: { content: [{ type: "text", text: "ok" }] },
    post: (message) => sent.push(message),
    remove: (reason) => removed.push(reason),
    callTool: async (name, args) => {
      calls.push({ name, args });
      return { result: { content: [{ type: "text", text: "called" }] } };
    },
    message: async () => true,
    openLink: async () => true,
    size: (height) => sizes.push(height),
    now: () => clock,
    ...overrides,
  });
  const tick = (ms: number): void => {
    clock += ms;
  };
  const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));
  return { bridge, sent, removed, calls, sizes, tick, settle };
}

const request = (id: number, method: string, params: unknown = {}) => ({
  jsonrpc: "2.0",
  id,
  method,
  params,
});
const notification = (method: string, params: unknown = {}) => ({ jsonrpc: "2.0", method, params });

async function initialized(h: ReturnType<typeof harness>): Promise<void> {
  h.bridge.receive(request(1, "ui/initialize", { protocolVersion: MCP_APPS_PROTOCOL }));
  await h.settle();
  h.bridge.receive(notification("ui/notifications/initialized"));
  await h.settle();
}

describe("MCP Apps bridge", () => {
  test("initialize answers with granted capabilities; input and result replay only after initialized", async () => {
    const h = harness();
    h.bridge.receive(request(1, "ui/initialize"));
    await h.settle();
    expect(h.sent).toHaveLength(1);
    const result = h.sent[0]?.["result"] as Record<string, unknown>;
    expect(result["protocolVersion"]).toBe(MCP_APPS_PROTOCOL);
    expect(result["hostInfo"]).toEqual({ name: "Cowboy", version: "test" });
    expect(result["hostCapabilities"]).toEqual({ logging: {}, serverTools: {} });
    // Requests before `initialized` are refused and nothing is replayed.
    h.bridge.receive(request(2, "tools/call", { name: "suger_status" }));
    await h.settle();
    expect(h.calls).toHaveLength(0);
    expect(err(h.sent[1]).message).toBe("not initialized");
    h.bridge.receive(notification("ui/notifications/initialized"));
    expect(h.sent.slice(2).map((message) => message["method"])).toEqual([
      "ui/notifications/tool-input",
      "ui/notifications/tool-result",
    ]);
    expect(h.sent[2]?.["params"]).toEqual({ arguments: { q: 1 } });
    // A second initialize is refused, never re-replayed.
    h.bridge.receive(notification("ui/notifications/initialized"));
    expect(h.sent).toHaveLength(4);
  });

  test("granted tools are called; others, unknown methods and ungranted capabilities are refused", async () => {
    const h = harness();
    await initialized(h);
    h.sent.length = 0;
    h.bridge.receive(request(3, "tools/list"));
    h.bridge.receive(request(4, "tools/call", { name: "suger_status", arguments: { a: 1 } }));
    h.bridge.receive(request(5, "tools/call", { name: "suger_write" }));
    h.bridge.receive(request(6, "sampling/createMessage"));
    h.bridge.receive(request(7, "ui/message", { content: [{ type: "text", text: "hi" }] }));
    h.bridge.receive(request(8, "ui/open-link", { url: "https://example.com" }));
    await h.settle();
    const byId = new Map(h.sent.map((message) => [message["id"], message]));
    expect(byId.get(3)?.["result"]).toEqual({
      tools: [{ name: "suger_status", inputSchema: { type: "object" } }],
    });
    expect(h.calls).toEqual([{ name: "suger_status", args: { a: 1 } }]);
    expect(byId.get(4)?.["result"]).toEqual({ content: [{ type: "text", text: "called" }] });
    expect(err(byId.get(5)).message).toBe("tool not granted");
    expect(err(byId.get(6)).code).toBe(-32601);
    expect(err(byId.get(7)).message).toBe("not granted");
    expect(err(byId.get(8)).message).toBe("not granted");
    expect(h.removed).toEqual([]);
  });

  test("a granted message takes text blocks only", async () => {
    const texts: string[] = [];
    const h = harness({
      capabilities: { message: true },
      message: async (text) => {
        texts.push(text);
        return true;
      },
    });
    await initialized(h);
    h.sent.length = 0;
    h.bridge.receive(request(9, "ui/message", { content: [{ type: "image", data: "x" }] }));
    h.bridge.receive(
      request(10, "ui/message", {
        content: [
          { type: "text", text: "a" },
          { type: "text", text: "b" },
        ],
      }),
    );
    await h.settle();
    expect(texts).toEqual(["a\nb"]);
    expect(h.sent.find((message) => message["id"] === 10)?.["result"]).toEqual({});
  });

  test("malformed messages, a message flood, a byte flood and a request flood remove the frame", async () => {
    for (const bad of [
      null,
      "text",
      { jsonrpc: "1.0", method: "ping" },
      { jsonrpc: "2.0", id: {}, method: "ping" },
    ]) {
      const h = harness();
      h.bridge.receive(bad);
      expect(h.removed).toEqual(["This view sent malformed data."]);
    }
    const messages = harness();
    for (let index = 0; index <= INGRESS_MESSAGES_PER_SECOND; index += 1) {
      messages.bridge.receive(notification("ui/notifications/size-changed", { height: index }));
    }
    expect(messages.removed).toHaveLength(1);
    // Spread over seconds the same count is fine.
    const paced = harness();
    for (let index = 0; index <= INGRESS_MESSAGES_PER_SECOND * 2; index += 1) {
      paced.bridge.receive(notification("ui/notifications/size-changed", { height: 10 }));
      if (index % 60 === 0) paced.tick(1000);
    }
    expect(paced.removed).toEqual([]);
    const bytes = harness();
    bytes.bridge.receive(
      notification("notifications/message", { data: "x".repeat(INGRESS_BYTES_PER_SECOND) }),
    );
    expect(bytes.removed).toHaveLength(1);
    const requests = harness();
    for (let index = 0; index <= INGRESS_REQUESTS_PER_SECOND; index += 1) {
      requests.bridge.receive(request(index, "ping"));
    }
    expect(requests.removed).toEqual(["This view stopped: it sent requests too fast."]);
  });

  test("sizes are clamped, teardown is asked once initialized, and an ended bridge is silent", async () => {
    const h = harness();
    h.bridge.receive(notification("ui/notifications/size-changed", { height: 99999 }));
    h.bridge.receive(notification("ui/notifications/size-changed", { height: -5 }));
    expect(h.sizes).toEqual([4000, 0]);
    h.bridge.teardown();
    expect(h.sent).toHaveLength(0);
    await initialized(h);
    h.sent.length = 0;
    h.bridge.teardown();
    expect(h.sent[0]?.["method"]).toBe("ui/resource-teardown");
    h.bridge.end();
    h.bridge.receive(request(20, "ping"));
    await h.settle();
    expect(h.sent).toHaveLength(1);
    const asked = harness();
    asked.bridge.receive(notification("ui/notifications/request-teardown"));
    expect(asked.removed).toEqual(["closed"]);
  });
});

describe("ingress measurement", () => {
  test("only plain JSON is measured; binary and exotic clones are refused", () => {
    expect(jsonBytes({ a: "é" }, 1000)).toBeGreaterThan(0);
    for (const exotic of [
      new ArrayBuffer(8),
      new Uint8Array(8),
      new Map(),
      new Date(),
      { nested: [new ArrayBuffer(1)] },
      Number.NaN,
    ]) {
      expect(jsonBytes(exotic, 1000)).toBe(Number.POSITIVE_INFINITY);
    }
    const cyclic: Record<string, unknown> = {};
    cyclic["self"] = cyclic;
    expect(jsonBytes(cyclic, 1000)).toBe(Number.POSITIVE_INFINITY);
    expect(jsonBytes("x".repeat(2000), 1000)).toBe(Number.POSITIVE_INFINITY);
  });

  test("a binary payload removes the frame instead of hiding in the log", () => {
    const h = harness();
    h.bridge.receive({
      jsonrpc: "2.0",
      method: "notifications/message",
      params: { data: new ArrayBuffer(64 * 1024 * 1024) },
    });
    expect(h.removed).toHaveLength(1);
    expect(h.bridge.log).toHaveLength(0);
  });
});

describe("MCP Apps admission", () => {
  const chrome = (platform: string, version: string) => ({
    navigator: {
      userAgentData: { mobile: false, platform, brands: [{ brand: "Google Chrome", version }] },
    } as unknown as Navigator & { userAgentData: unknown },
  });
  test("its own table: macOS Chrome from 157, nothing else", () => {
    expect(mcpAppsEnabledFor(chrome("macOS", "157") as never)).toBe(true);
    expect(mcpAppsEnabledFor(chrome("macOS", "156") as never)).toBe(false);
    expect(mcpAppsEnabledFor(chrome("Linux", "200") as never)).toBe(false);
    expect(mcpAppsEnabledFor(chrome("Windows", "200") as never)).toBe(false);
  });
});
