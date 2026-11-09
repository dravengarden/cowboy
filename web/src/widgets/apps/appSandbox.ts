// Host side of one app runtime (docs/widget-apps.md §4.2, §5): the srcdoc
// sandbox frame, the readiness and nonce handshake, the heartbeat and credit
// flow. Everything the runtime sends arrives already bounded by the trusted
// Worker; this module still treats it as untrusted data.

import { vmSource, workerSource } from "virtual:cowboy-app-runtime";
import wasmUrl from "@jitl/quickjs-wasmfile-release-sync/wasm?url";
import { APP_SANDBOX_BOOT, APP_SANDBOX_CSP } from "./appSandboxBoot";
import { LIMITS } from "./runtime/limits.js";

const SANDBOX_DOCUMENT = `<!doctype html><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${APP_SANDBOX_CSP}">
<script>${APP_SANDBOX_BOOT}</script>`;

const READY_TIMEOUT_MS = 5000;
const HEARTBEAT_MS = 1000;
const MISSED_HEARTBEATS = 3;

let wasmBytes: Promise<ArrayBuffer> | null = null;
function wasm(): Promise<ArrayBuffer> {
  wasmBytes ??= fetch(wasmUrl, { credentials: "omit" }).then((response) => {
    if (!response.ok) throw new Error("the app runtime could not load");
    return response.arrayBuffer();
  });
  return wasmBytes;
}

export type AppRuntimeEvents = {
  /** A validated render batch (JSON text of operations). */
  onOps: (json: string) => void;
  /** The app asked to send a message; the host must confirm before anything happens. */
  onSend: (text: string) => void;
  onLog?: (text: string) => void;
  /** The runtime ended; `reason` is person-readable. */
  onStopped: (reason: string) => void;
};

export type AppRuntime = {
  event: (id: number, name: string, payload: unknown) => void;
  props: (props: unknown) => void;
  context: (context: unknown) => void;
  /** Allow one more render batch (granted per animation frame by the host). */
  credit: () => void;
  terminate: () => void;
};

export type AppRuntimeStart = {
  code: string;
  props: unknown;
  context: unknown;
};

let generation = 0;

export function startAppRuntime(start: AppRuntimeStart, events: AppRuntimeEvents): AppRuntime {
  const frame = document.createElement("iframe");
  const frameGeneration = ++generation;
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("allow", "");
  frame.setAttribute("aria-hidden", "true");
  frame.tabIndex = -1;
  frame.dataset["cowboyAppSandbox"] = String(frameGeneration);
  frame.style.cssText =
    "position:fixed;width:0;height:0;border:0;opacity:0;pointer-events:none;inset:auto";
  frame.srcdoc = SANDBOX_DOCUMENT;
  const channel = new MessageChannel();
  const nonce = crypto.randomUUID();
  let closed = false;
  let verified = false;
  let booted = false;
  let missed = 0;
  let pingNumber = 0;

  const end = (reason: string): void => {
    if (closed) return;
    closed = true;
    globalThis.removeEventListener("message", onWindowMessage);
    clearInterval(heartbeat);
    clearTimeout(readyTimer);
    channel.port1.close();
    frame.remove();
    events.onStopped(reason);
  };
  const onWindowMessage = (event: MessageEvent): void => {
    if (closed || event.source !== frame.contentWindow) return;
    const data = event.data as { t?: unknown; error?: unknown } | null;
    if (data?.t === "cowboy-app-sandbox-ready" && !booted) {
      booted = true;
      void wasm().then(
        (bytes) => {
          if (closed) return;
          const copy = bytes.slice(0);
          frame.contentWindow?.postMessage(
            {
              t: "cowboy-app-boot",
              nonce,
              worker: workerSource,
              vm: vmSource,
              wasm: copy,
              ...start,
            },
            "*",
            [channel.port2, copy],
          );
        },
        () => end("The app runtime could not load."),
      );
    } else if (data?.t === "cowboy-app-sandbox-error") {
      end(`The app failed to start: ${String(data.error ?? "unknown error").slice(0, 200)}`);
    }
  };
  channel.port1.onmessage = (event: MessageEvent): void => {
    if (closed) return;
    const message = event.data as Record<string, unknown> | null;
    if (!message || typeof message !== "object") return end("The app sent malformed data.");
    if (!verified) {
      // The first message must prove this port belongs to this frame's boot.
      if (message["t"] === "nonce" && message["nonce"] === nonce) verified = true;
      else if (message["t"] === "stopped") end(`This app stopped: ${String(message["reason"])}.`);
      else end("The app runtime could not be verified.");
      return;
    }
    switch (message["t"]) {
      case "ops":
        if (typeof message["json"] === "string") events.onOps(message["json"]);
        return;
      case "send":
        if (typeof message["text"] === "string") events.onSend(message["text"]);
        return;
      case "log":
        if (typeof message["text"] === "string") events.onLog?.(message["text"]);
        return;
      case "pong":
        missed = 0;
        return;
      case "stopped":
        end(`This app stopped: ${String(message["reason"]).slice(0, 200)}.`);
        return;
      default:
        end("The app sent malformed data.");
    }
  };
  const readyTimer = setTimeout(() => {
    if (!verified) end("The app did not start in time.");
  }, READY_TIMEOUT_MS + LIMITS.loadMs);
  const heartbeat = setInterval(() => {
    if (!verified) return;
    missed += 1;
    if (missed > MISSED_HEARTBEATS) end("This app stopped responding.");
    else channel.port1.postMessage({ t: "ping", n: ++pingNumber });
  }, HEARTBEAT_MS);
  globalThis.addEventListener("message", onWindowMessage);
  document.body.append(frame);

  const post = (message: unknown): void => {
    if (!closed && verified) channel.port1.postMessage(message);
  };
  return {
    event: (id, name, payload) => post({ t: "event", id, name, payload }),
    props: (props) => post({ t: "props", props }),
    context: (context) => post({ t: "context", context }),
    credit: () => post({ t: "credit" }),
    terminate: () => {
      post({ t: "stop" });
      end("closed");
    },
  };
}
