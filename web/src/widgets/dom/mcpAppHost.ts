// Host side of one signed MCP Apps view (docs/widget-apps-mcp.md §5-§7): the
// frame, its server-side lease and the Controller calls behind the bridge.
//
// - The frame is exactly `sandbox="allow-scripts"` and loads a single-use
//   `/mcp-app/<token>.html`; its first `load` is the view and any later one
//   removes the frame (a navigated document never inherits the bridge).
// - Messages are taken only from that frame's window with origin "null".
// - The lease is renewed every 10 s; a refused renewal removes the frame.
//   Effects need the lease on the Controller, so a stalled page cannot keep
//   one alive: the parent's timers are never the authority.

import {
  McpBridge,
  type McpAppCapabilities,
  type McpAppHostContext,
  type ToolOutcome,
} from "./mcpAppBridge";
import { DOM_APP_SANDBOX } from "./domAppHost";

const RENEW_MS = 10_000;
const READY_TIMEOUT_MS = 10_000;
const FEATURES = [
  "accelerometer", "autoplay", "bluetooth", "camera", "clipboard-read", "clipboard-write",
  "display-capture", "encrypted-media", "fullscreen", "geolocation", "gyroscope", "hid",
  "microphone", "midi", "payment", "publickey-credentials-get", "serial", "usb", "web-share",
];

export type McpAppFrame = {
  url: string;
  lease: string;
  capabilities: McpAppCapabilities;
};

type Reply = { status: number; body: Record<string, unknown> };
type Transport = (method: "POST" | "DELETE", url: string, body?: unknown) => Promise<Reply>;

const fetchTransport: Transport = async (method, url, body) => {
  const response = await fetch(
    url,
    body === undefined
      ? { method }
      : { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) },
  );
  return {
    status: response.status,
    body: (await response.json().catch(() => ({}))) as Record<string, unknown>,
  };
};

let transport: Transport = fetchTransport;

/** Fixtures substitute the Controller. */
export function setMcpAppTransport(next: Transport | null): void {
  transport = next ?? fetchTransport;
}

const part = encodeURIComponent;
const framesUrl = (session: string): string => `/api/sessions/${part(session)}/mcp-apps/frames`;

const REFUSALS: Readonly<Record<string, string>> = {
  no_grant: "This view is not granted in this session.",
  no_binding: "This view's server is no longer bound to this session.",
  undeclared_view: "This server release no longer declares this view.",
  undeclared_tool: "The grant names a tool this release does not offer to views.",
  too_many_frames: "Too many views are open. Stop one, then run this again.",
  release_unavailable: "This server release is not available on the Controller.",
};

/** Opens a frame lease for `binding`'s view `uri`. */
export async function openMcpAppFrame(
  session: string,
  binding: string,
  uri: string,
): Promise<McpAppFrame | { refused: string }> {
  let reply: Reply;
  try {
    reply = await transport("POST", framesUrl(session), { binding, uri });
  } catch {
    return { refused: "The Controller could not be reached." };
  }
  const { status, body } = reply;
  if (status === 200 && typeof body["url"] === "string" && typeof body["lease"] === "string") {
    return {
      url: body["url"],
      lease: body["lease"],
      capabilities: (body["capabilities"] ?? {}) as McpAppCapabilities,
    };
  }
  const code = typeof body["error"] === "string" ? body["error"] : "";
  return { refused: REFUSALS[code] ?? "This view cannot run now." };
}

export type McpAppConfirm = (call: { tool: string; arguments: unknown }) => Promise<boolean>;

export type McpAppStart = {
  session: string;
  frame: McpAppFrame;
  hostContext: McpAppHostContext;
  version: string;
  toolInput: Record<string, unknown>;
  toolResult: unknown;
};

export type McpAppEvents = {
  onHeight: (height: number) => void;
  /** A call that is not read-only: the person must confirm tool and arguments. */
  confirmCall: McpAppConfirm;
  /** `ui/message`: opens the product confirmation; false if refused at once. */
  message: (text: string, lease: string) => boolean;
  openLink: (url: string) => void;
  onStopped: (reason: string) => void;
};

export type McpAppRun = {
  frame: HTMLIFrameElement;
  stop: (reason?: string) => void;
  hostContextChanged: (context: Partial<McpAppHostContext>) => void;
};

async function callTool(
  start: McpAppStart,
  events: McpAppEvents,
  name: string,
  args: Record<string, unknown>,
): Promise<ToolOutcome> {
  const url = `${framesUrl(start.session)}/${part(start.frame.lease)}/call`;
  const refused = (message: string): ToolOutcome => ({ error: { code: -32000, message } });
  try {
    let reply = await transport("POST", url, { name, arguments: args });
    if (reply.status === 200 && reply.body["state"] === "confirm") {
      if (!(await events.confirmCall({ tool: name, arguments: reply.body["arguments"] }))) {
        return refused("The person declined this call.");
      }
      reply = await transport("POST", url, {
        name,
        arguments: args,
        confirmed: reply.body["digest"],
      });
    }
    const { status, body } = reply;
    if (status === 200 && body["state"] === "ok" && typeof body["response"] === "object") {
      const response = body["response"] as Record<string, unknown> | null;
      if (response && "result" in response) return { result: response["result"] };
      const error = response?.["error"] as { code?: unknown; message?: unknown } | undefined;
      return {
        error: {
          code: typeof error?.code === "number" ? error.code : -32603,
          message: typeof error?.message === "string" ? error.message.slice(0, 500) : "failed",
        },
      };
    }
    if (body["uncertain"] === true) {
      return refused("The call's outcome is unknown; Cowboy does not retry it.");
    }
    return refused(typeof body["error"] === "string" ? body["error"] : "refused");
  } catch {
    return refused("The call's outcome is unknown; Cowboy does not retry it.");
  }
}

/** Mounts the view's frame into `container` under the lease in `start`. */
export function startMcpApp(
  container: HTMLElement,
  start: McpAppStart,
  events: McpAppEvents,
): McpAppRun {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", DOM_APP_SANDBOX);
  frame.setAttribute("allow", FEATURES.map((feature) => `${feature} 'none'`).join("; "));
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.setAttribute("title", "MCP app view");
  frame.style.cssText = "display:block;width:100%;height:160px;border:0;background:transparent";
  let ended = false;
  let loads = 0;
  let pendingHeight: number | null = null;
  let sizeFrame = 0;
  const lease = start.frame.lease;
  const leaseUrl = `${framesUrl(start.session)}/${part(lease)}`;

  const end = (reason: string): void => {
    if (ended) return;
    ended = true;
    bridge.teardown();
    bridge.end();
    globalThis.removeEventListener("message", onWindowMessage);
    clearInterval(renewal);
    clearTimeout(readyTimer);
    cancelAnimationFrame(sizeFrame);
    frame.remove();
    void transport("DELETE", leaseUrl).catch(() => undefined);
    events.onStopped(reason);
  };

  const bridge = new McpBridge({
    capabilities: start.frame.capabilities,
    hostContext: start.hostContext,
    version: start.version,
    toolInput: start.toolInput,
    toolResult: start.toolResult,
    post: (message) => {
      // An opaque origin can only be addressed with "*"; the frame's window
      // is the one this host created and nothing else receives it.
      if (!ended) frame.contentWindow?.postMessage(message, "*");
    },
    remove: end,
    callTool: (name, args) => callTool(start, events, name, args),
    message: async (text) => events.message(text, lease),
    openLink: async (url) => {
      try {
        const reply = await transport("POST", `${leaseUrl}/open-link`, { url });
        if (reply.status !== 200 || typeof reply.body["url"] !== "string") return false;
        events.openLink(reply.body["url"]);
        return true;
      } catch {
        return false;
      }
    },
    // Coalesced to one layout update per animation frame.
    size: (height) => {
      pendingHeight = height;
      if (sizeFrame) return;
      sizeFrame = requestAnimationFrame(() => {
        sizeFrame = 0;
        if (pendingHeight !== null && !ended) events.onHeight(pendingHeight);
        pendingHeight = null;
      });
    },
  });

  const onWindowMessage = (event: MessageEvent): void => {
    if (ended || event.source !== frame.contentWindow) return;
    if (event.origin !== "null") {
      end("This view stopped: it sent unexpected messages.");
      return;
    }
    bridge.receive(event.data);
  };
  // The first load is the view; any later one is the view navigating its
  // frame, which never inherits the bridge.
  frame.addEventListener("load", () => {
    loads += 1;
    if (loads > 1) end("This view stopped: it tried to navigate.");
  });
  const readyTimer = setTimeout(() => {
    if (!bridge.running) end("The view did not start in time.");
  }, READY_TIMEOUT_MS);
  const renewal = setInterval(() => {
    void transport("POST", `${leaseUrl}/renew`).then(
      (reply) => {
        if (reply.status !== 200) end("This view's permission ended.");
      },
      () => end("Lost the connection to the Controller."),
    );
  }, RENEW_MS);
  globalThis.addEventListener("message", onWindowMessage);
  frame.src = start.frame.url;
  container.append(frame);
  return {
    frame,
    stop: (reason = "closed") => end(reason),
    hostContextChanged: (context) => bridge.hostContextChanged(context),
  };
}
