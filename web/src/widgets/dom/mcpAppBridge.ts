// The MCP Apps bridge of one signed view (docs/widget-apps-mcp.md §7): the
// view speaks JSON-RPC 2.0 to its parent. This module is the protocol only;
// the frame, lease and Controller calls are the host's (mcpAppHost.ts), so
// every rule here is testable without a browser.
//
// - Ingress is budgeted on arrival, before anything but its size is read:
//   120 messages and 256 KiB per second, requests also 30 per second.
//   Exceeding either, or a malformed message, removes the frame.
// - Nothing is sent to the view before it answers `ui/initialize` with
//   `ui/notifications/initialized`; then the tool input and result replay.
// - Unknown requests get method-not-found; unknown notifications are ignored.

/** The pinned MCP Apps revision (SEP-1865, final 2026-01-26). */
export const MCP_APPS_PROTOCOL = "2026-01-26";

export const INGRESS_MESSAGES_PER_SECOND = 120;
export const INGRESS_BYTES_PER_SECOND = 256 * 1024;
export const INGRESS_REQUESTS_PER_SECOND = 30;
const LOG_ENTRIES = 50;

export type McpAppCapabilities = {
  call_tools?: string[];
  message?: boolean;
  open_links?: boolean;
};

export type JsonRpcError = { code: number; message: string };
export type ToolOutcome = { result: unknown } | { error: JsonRpcError };

export type McpAppHostContext = {
  theme: "light" | "dark";
  locale: string;
  platform: "web";
  containerDimensions?: { width: number; maxHeight: number };
};

export type McpBridgeHost = {
  capabilities: McpAppCapabilities;
  hostContext: McpAppHostContext;
  version: string;
  /** The agent's call this view was opened for, replayed after initialize. */
  toolInput: Record<string, unknown>;
  toolResult: unknown;
  post: (message: Record<string, unknown>) => void;
  remove: (reason: string) => void;
  callTool: (name: string, args: Record<string, unknown>) => Promise<ToolOutcome>;
  /** Opens the product confirmation; false when it was refused at once. */
  message: (text: string) => Promise<boolean>;
  openLink: (url: string) => Promise<boolean>;
  size: (height: number) => void;
  now?: () => number;
};

const NOT_FOUND = -32601;
const INVALID = -32602;
const REFUSED = -32000;

type Message = Record<string, unknown>;

const isObject = (value: unknown): value is Message =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const validId = (id: unknown): id is string | number =>
  (typeof id === "string" && id.length <= 128) || (typeof id === "number" && Number.isSafeInteger(id));

/** UTF-8 bytes of `data` as JSON, or Infinity when it is not plain JSON
 *  (binary buffers, Maps, Blobs, Dates, cycles...) or exceeds `limit`.
 *  Structured clone can carry what JSON.stringify would silently shrink, so
 *  the walk stops early instead of measuring a serialization. */
export function jsonBytes(data: unknown, limit: number): number {
  let total = 0;
  const seen = new Set<object>();
  const encoder = new TextEncoder();
  const walk = (value: unknown, depth: number): boolean => {
    if (total > limit || depth > 64) return false;
    if (value === null || typeof value === "boolean") {
      total += 5;
      return true;
    }
    if (typeof value === "number") {
      if (!Number.isFinite(value)) return false;
      total += 24;
      return true;
    }
    if (typeof value === "string") {
      total += value.length > limit ? value.length : encoder.encode(value).length + 2;
      return total <= limit;
    }
    if (typeof value !== "object" || seen.has(value)) return false;
    seen.add(value);
    if (Array.isArray(value)) {
      total += 2;
      for (const item of value) if (!walk(item, depth + 1)) return false;
      return true;
    }
    const proto = Object.getPrototypeOf(value);
    if (proto !== Object.prototype && proto !== null) return false;
    total += 2;
    for (const [key, item] of Object.entries(value)) {
      total += key.length + 3;
      if (!walk(item, depth + 1)) return false;
    }
    return true;
  };
  return walk(data, 0) && total <= limit ? total : Number.POSITIVE_INFINITY;
}

export class McpBridge {
  private readonly host: McpBridgeHost;
  private readonly now: () => number;
  private windowStart = 0;
  private messages = 0;
  private bytes = 0;
  private requests = 0;
  private initializeAnswered = false;
  private initialized = false;
  private ended = false;
  private nextId = 1;
  /** Bounded record of the view's `notifications/message`. */
  readonly log: Array<{ level: string; data: unknown }> = [];

  constructor(host: McpBridgeHost) {
    this.host = host;
    this.now = host.now ?? (() => performance.now());
  }

  get running(): boolean {
    return this.initialized && !this.ended;
  }

  /** Ends the bridge; later messages are dropped. */
  end(): void {
    this.ended = true;
  }

  /** Asks the view to clean up before it is removed (best effort). */
  teardown(): void {
    if (this.initialized && !this.ended) {
      this.host.post({ jsonrpc: "2.0", id: this.id(), method: "ui/resource-teardown", params: {} });
    }
  }

  hostContextChanged(context: Partial<McpAppHostContext>): void {
    if (this.running) this.notify("ui/notifications/host-context-changed", context);
  }

  /** One message from the view's frame (source and origin already checked). */
  receive(data: unknown): void {
    if (this.ended) return;
    if (!this.admit(data)) {
      this.fail("This view stopped: it sent too much or unreadable data.");
      return;
    }
    if (!isObject(data) || data["jsonrpc"] !== "2.0") {
      this.fail("This view sent malformed data.");
      return;
    }
    const method = data["method"];
    if (typeof method !== "string") {
      // A response to a host request (teardown, ping): nothing waits on it.
      if (!validId(data["id"])) this.fail("This view sent malformed data.");
      return;
    }
    const params = data["params"] === undefined ? {} : data["params"];
    if (!isObject(params)) {
      this.fail("This view sent malformed data.");
      return;
    }
    if (!("id" in data)) {
      this.notification(method, params);
      return;
    }
    const id = data["id"];
    if (!validId(id)) {
      this.fail("This view sent malformed data.");
      return;
    }
    if ((this.requests += 1) > INGRESS_REQUESTS_PER_SECOND) {
      this.fail("This view stopped: it sent requests too fast.");
      return;
    }
    void this.request(id, method, params);
  }

  private admit(data: unknown): boolean {
    const now = this.now();
    if (now - this.windowStart >= 1000) {
      this.windowStart = now;
      this.messages = 0;
      this.bytes = 0;
      this.requests = 0;
    }
    this.messages += 1;
    this.bytes += jsonBytes(data, INGRESS_BYTES_PER_SECOND - this.bytes);
    return this.messages <= INGRESS_MESSAGES_PER_SECOND && this.bytes <= INGRESS_BYTES_PER_SECOND;
  }

  private fail(reason: string): void {
    if (this.ended) return;
    this.ended = true;
    this.host.remove(reason);
  }

  private id(): string {
    return `cowboy-${this.nextId++}`;
  }

  private notify(method: string, params: unknown): void {
    if (!this.ended) this.host.post({ jsonrpc: "2.0", method, params });
  }

  private reply(id: string | number, outcome: ToolOutcome): void {
    if (this.ended) return;
    this.host.post(
      "error" in outcome
        ? { jsonrpc: "2.0", id, error: outcome.error }
        : { jsonrpc: "2.0", id, result: outcome.result },
    );
  }

  private notification(method: string, params: Message): void {
    switch (method) {
      case "ui/notifications/initialized":
        if (!this.initializeAnswered || this.initialized) return;
        this.initialized = true;
        this.notify("ui/notifications/tool-input", { arguments: this.host.toolInput });
        this.notify("ui/notifications/tool-result", this.host.toolResult);
        return;
      case "ui/notifications/size-changed": {
        const height = params["height"];
        if (typeof height === "number" && Number.isFinite(height)) {
          this.host.size(Math.max(0, Math.min(4000, Math.round(height))));
        }
        return;
      }
      case "ui/notifications/request-teardown":
        this.fail("closed");
        return;
      case "notifications/message":
        if (this.log.length >= LOG_ENTRIES) this.log.shift();
        this.log.push({
          level: typeof params["level"] === "string" ? params["level"].slice(0, 16) : "info",
          data: jsonBytes(params["data"], 2048) <= 2048 ? params["data"] : "[truncated]",
        });
        return;
      default:
        return;
    }
  }

  private async request(id: string | number, method: string, params: Message): Promise<void> {
    if (method === "ping") {
      this.reply(id, { result: {} });
      return;
    }
    if (method === "ui/initialize") {
      if (this.initializeAnswered) {
        this.reply(id, { error: { code: INVALID, message: "already initialized" } });
        return;
      }
      this.initializeAnswered = true;
      this.reply(id, { result: this.initializeResult() });
      return;
    }
    if (!this.initialized) {
      this.reply(id, { error: { code: INVALID, message: "not initialized" } });
      return;
    }
    const capabilities = this.host.capabilities;
    switch (method) {
      case "tools/list":
        this.reply(id, {
          result: {
            tools: (capabilities.call_tools ?? []).map((name) => ({
              name,
              inputSchema: { type: "object" },
            })),
          },
        });
        return;
      case "tools/call": {
        const name = params["name"];
        const args = params["arguments"] === undefined ? {} : params["arguments"];
        if (typeof name !== "string" || !isObject(args)) {
          this.reply(id, { error: { code: INVALID, message: "invalid tools/call" } });
          return;
        }
        if (!(capabilities.call_tools ?? []).includes(name)) {
          this.reply(id, { error: { code: REFUSED, message: "tool not granted" } });
          return;
        }
        this.reply(id, await this.host.callTool(name, args));
        return;
      }
      case "ui/message": {
        const text = messageText(params);
        if (!capabilities.message || text === null) {
          this.reply(id, {
            error: { code: REFUSED, message: capabilities.message ? "text only" : "not granted" },
          });
          return;
        }
        const opened = await this.host.message(text);
        this.reply(
          id,
          opened ? { result: {} } : { error: { code: REFUSED, message: "not sent" } },
        );
        return;
      }
      case "ui/open-link": {
        const url = params["url"];
        if (!capabilities.open_links || typeof url !== "string") {
          this.reply(id, { error: { code: REFUSED, message: "not granted" } });
          return;
        }
        const opened = await this.host.openLink(url);
        this.reply(id, opened ? { result: {} } : { error: { code: REFUSED, message: "refused" } });
        return;
      }
      default:
        this.reply(id, { error: { code: NOT_FOUND, message: "method not found" } });
    }
  }

  private initializeResult(): Record<string, unknown> {
    const capabilities = this.host.capabilities;
    return {
      protocolVersion: MCP_APPS_PROTOCOL,
      hostInfo: { name: "Cowboy", version: this.host.version },
      hostCapabilities: {
        logging: {},
        ...((capabilities.call_tools ?? []).length > 0 ? { serverTools: {} } : {}),
        ...(capabilities.message ? { message: { text: {} } } : {}),
        ...(capabilities.open_links ? { openLinks: {} } : {}),
      },
      hostContext: this.host.hostContext,
    };
  }
}

/** The text of a `ui/message`: one or more text blocks, nothing else. */
function messageText(params: Message): string | null {
  const content = params["content"];
  if (!Array.isArray(content) || content.length === 0) return null;
  const parts: string[] = [];
  for (const block of content) {
    if (!isObject(block) || block["type"] !== "text" || typeof block["text"] !== "string") {
      return null;
    }
    parts.push(block["text"]);
  }
  return parts.join("\n");
}
