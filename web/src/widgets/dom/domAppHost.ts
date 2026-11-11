// Host side of one full-DOM app (docs/widget-apps-dom.md §3-§5): the
// same-origin shell in a frame that is always exactly `sandbox="allow-scripts"`
// (an opaque origin: allow-same-origin here would hand the app Cowboy's
// cookies, storage and API), a one-shot handshake bound to a fresh nonce and
// generation, and best-effort removal on floods, navigation or silence.

/** The only sandbox this frame ever gets (asserted by domAppHost.test.ts). */
export const DOM_APP_SANDBOX = "allow-scripts";
export const APP_SHELL_URL = "/app-shell.html";

const FEATURES = [
  "accelerometer", "ambient-light-sensor", "autoplay", "bluetooth", "camera",
  "clipboard-read", "clipboard-write", "display-capture", "encrypted-media",
  "fullscreen", "gamepad", "geolocation", "gyroscope", "hid", "idle-detection",
  "local-fonts", "magnetometer", "microphone", "midi", "payment",
  "picture-in-picture", "publickey-credentials-create", "publickey-credentials-get",
  "screen-wake-lock", "serial", "usb", "web-share", "window-management",
];
const READY_TIMEOUT_MS = 5000;
const HEARTBEAT_MS = 1000;
const MISSED_HEARTBEATS = 3;
const PORT_MESSAGES_PER_SECOND = 30;

export type DomAppStart = {
  code: string;
  props: unknown;
  images: Readonly<Record<string, Uint8Array>>;
  context: unknown;
};

export type DomAppEvents = {
  onBooted: () => void;
  onHeight: (height: number) => void;
  /** The app asked to send; the host must confirm before anything happens. */
  onSend: (text: string) => void;
  /** The frame is gone; `reason` is person-readable. */
  onStopped: (reason: string) => void;
};

export type DomApp = {
  frame: HTMLIFrameElement;
  props: (props: unknown) => void;
  stop: (reason?: string) => void;
};

/** Mounts the app's frame into `container` and starts it. */
export function startDomApp(
  container: HTMLElement,
  start: DomAppStart,
  events: DomAppEvents,
): DomApp {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", DOM_APP_SANDBOX);
  frame.setAttribute("allow", FEATURES.map((feature) => `${feature} 'none'`).join("; "));
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.setAttribute("title", "App");
  frame.style.cssText = "display:block;width:100%;height:120px;border:0;background:transparent";
  frame.src = APP_SHELL_URL;
  const nonce = crypto.randomUUID();
  const generation = crypto.randomUUID();
  let port: MessagePort | null = null;
  let handshaken = false;
  let ended = false;
  let loads = 0;
  let missed = 0;
  let ping = 0;
  let windowStart = performance.now();
  let windowCount = 0;
  // Props that arrive before the app has drawn are replayed after it boots.
  let latestProps = start.props;
  let bootedProps: unknown = undefined;
  let booted = false;

  const end = (reason: string): void => {
    if (ended) return;
    ended = true;
    globalThis.removeEventListener("message", onWindowMessage);
    clearTimeout(readyTimer);
    clearInterval(heartbeat);
    port?.close();
    frame.remove();
    events.onStopped(reason);
  };
  const onWindowMessage = (event: MessageEvent): void => {
    if (ended || event.source !== frame.contentWindow) return;
    const data = event.data as { t?: unknown } | null;
    // Exactly one `ready` from this frame's opaque document; anything else
    // through the window bypasses the port and removes the app (§5).
    if (handshaken || event.origin !== "null" || data?.t !== "cowboy-dom-ready") {
      end("This app stopped: it sent unexpected messages.");
      return;
    }
    handshaken = true;
    const channel = new MessageChannel();
    port = channel.port1;
    port.onmessage = (message: MessageEvent) => void onPort(message.data);
    // An opaque origin can only be addressed with "*"; the port is bound by
    // the nonce and generation the shell must echo first.
    frame.contentWindow?.postMessage({ t: "cowboy-dom-port", nonce, generation }, "*", [
      channel.port2,
    ]);
  };
  let verified = false;
  const onPort = async (message: unknown): Promise<void> => {
    if (ended) return;
    const now = performance.now();
    if (now - windowStart >= 1000) {
      windowStart = now;
      windowCount = 0;
    }
    if ((windowCount += 1) > PORT_MESSAGES_PER_SECOND) {
      end("This app stopped: it sent messages too fast.");
      return;
    }
    if (!message || typeof message !== "object") return end("This app sent malformed data.");
    const data = message as Record<string, unknown>;
    if (!verified) {
      if (data["t"] !== "nonce" || data["nonce"] !== nonce || data["generation"] !== generation) {
        end("The app could not be verified.");
        return;
      }
      verified = true;
      const { runtimeSource } = await import("virtual:cowboy-dom-runtime");
      if (ended) return;
      bootedProps = latestProps;
      port?.postMessage({ t: "boot", runtime: runtimeSource, ...start, props: bootedProps });
      return;
    }
    switch (data["t"]) {
      case "booted":
        booted = true;
        if (latestProps !== bootedProps) port?.postMessage({ t: "props", props: latestProps });
        events.onBooted();
        return;
      case "pong":
        missed = 0;
        return;
      case "height":
        if (typeof data["height"] === "number" && Number.isFinite(data["height"])) {
          events.onHeight(Math.max(0, Math.min(4000, Math.round(data["height"]))));
        }
        return;
      case "send":
        if (typeof data["text"] === "string") events.onSend(data["text"].slice(0, 8000));
        return;
      case "error":
        end(`This app stopped: ${String(data["text"]).slice(0, 200)}`);
        return;
      default:
        end("This app sent malformed data.");
    }
  };
  // The first load is the shell; any later one is the app navigating its
  // frame, which revokes this generation (§4).
  frame.addEventListener("load", () => {
    loads += 1;
    if (loads > 1) end("This app stopped: it tried to navigate.");
  });
  const readyTimer = setTimeout(() => {
    if (!verified) end("The app did not start in time.");
  }, READY_TIMEOUT_MS);
  const heartbeat = setInterval(() => {
    if (!verified || ended) return;
    if ((missed += 1) > MISSED_HEARTBEATS) end("This app stopped responding.");
    else port?.postMessage({ t: "ping", n: (ping += 1) });
  }, HEARTBEAT_MS);
  globalThis.addEventListener("message", onWindowMessage);
  container.append(frame);
  return {
    frame,
    props: (props) => {
      latestProps = props;
      if (booted && !ended) port?.postMessage({ t: "props", props });
    },
    stop: (reason = "closed") => end(reason),
  };
}
