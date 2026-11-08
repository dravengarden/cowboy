/** The viewer's half of `GET .../browser/stream`. One WebSocket carries RFB
 * bytes as binary frames and Cowboy control messages as text frames. noVNC is
 * handed a channel object instead of a URL: the channel owns the socket,
 * forwards binary frames to noVNC and routes every text frame to the viewer,
 * so noVNC never sees a control message. See docs/machine-browser.md,
 * "Implementation notes". */
import type { RfbChannel } from "@novnc/novnc";
import { type BrowserStateEvent, validBrowserState } from "./api";

export const BROWSER_STREAM_SUBPROTOCOL = "cowboy-browser-v1";
/** Product sockets close with this code when the Cowboy session must sign in again. */
export const AUTH_REQUIRED_CLOSE_CODE = 4001;
/** The Machine expires an attachment 30 s after its last renewal. */
export const LEASE_RENEWAL_MS = 10_000;

const CONNECTING = 0;
const OPEN = 1;

export type StreamControl =
  | { type: "state"; state: BrowserStateEvent }
  | { type: "error"; code: string }
  | { type: "auth_session"; session: unknown };

export interface StreamClose {
  code: number;
  reason: string;
  authRequired: boolean;
}

export interface StreamHandlers {
  onControl(message: StreamControl): void;
  onClose(close: StreamClose): void;
  onError?(): void;
}

/** The WebSocket surface the stream uses; tests supply a fake. */
export interface StreamSocket {
  binaryType: BinaryType;
  readonly protocol: string;
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  send(data: string | BufferSource): void;
  close(code?: number, reason?: string): void;
}

export interface BrowserStream {
  /** Pass to `new RFB(target, channel)`. */
  readonly channel: RfbChannel;
  sendLease(): boolean;
  sendResize(width: number, height: number): boolean;
  close(): void;
}

/** A text frame from the Controller; anything else is ignored. */
export function parseStreamControl(text: string): StreamControl | undefined {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return undefined;
  const message = value as Record<string, unknown>;
  switch (message.type) {
    case "state":
      return validBrowserState(message.state) ? { type: "state", state: message.state } : undefined;
    case "error":
      return typeof message.code === "string" && message.code.length > 0
        ? { type: "error", code: message.code }
        : undefined;
    case "auth_session":
      return { type: "auth_session", session: message.session };
    default:
      return undefined;
  }
}

type BinaryHandler = ((event: { data: ArrayBuffer }) => void) | null;

/** WebSocket-shaped object for noVNC's `Websock.attach`. Own properties and
 * prototype members together are exactly the names noVNC checks for. */
class Channel implements RfbChannel {
  onopen: ((event: Event) => void) | null = null;
  onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  #onmessage: BinaryHandler = null;
  #pending: ArrayBuffer[] = [];
  #flushScheduled = false;
  readonly #socket: StreamSocket;

  constructor(socket: StreamSocket) {
    this.#socket = socket;
  }

  get binaryType(): BinaryType {
    return this.#socket.binaryType;
  }
  set binaryType(value: BinaryType) {
    this.#socket.binaryType = value;
  }
  get protocol(): string {
    return this.#socket.protocol;
  }
  get readyState(): number {
    return this.#socket.readyState;
  }
  get onmessage(): BinaryHandler {
    return this.#onmessage;
  }
  set onmessage(handler: BinaryHandler) {
    this.#onmessage = handler;
    if (handler && this.#pending.length > 0) this.#scheduleFlush();
  }

  send(data: BufferSource): void {
    if (this.#socket.readyState === OPEN) this.#socket.send(data);
  }

  close(code?: number, reason?: string): void {
    const state = this.#socket.readyState;
    if (state === CONNECTING || state === OPEN) this.#socket.close(code, reason);
  }

  /** Binary frames that arrive before noVNC attaches wait, in order. */
  deliver(data: ArrayBuffer): void {
    if (this.#onmessage && this.#pending.length === 0 && !this.#flushScheduled) {
      this.#onmessage({ data });
      return;
    }
    this.#pending.push(data);
    this.#scheduleFlush();
  }

  #scheduleFlush(): void {
    if (this.#flushScheduled) return;
    this.#flushScheduled = true;
    // After noVNC's synchronous attach has also initialised its handshake state.
    queueMicrotask(() => {
      this.#flushScheduled = false;
      const handler = this.#onmessage;
      if (!handler) return;
      const frames = this.#pending;
      this.#pending = [];
      for (const data of frames) handler({ data });
    });
  }
}

export function createBrowserStream(socket: StreamSocket, handlers: StreamHandlers): BrowserStream {
  socket.binaryType = "arraybuffer";
  const channel = new Channel(socket);
  socket.onopen = (event) => {
    channel.onopen?.(event);
  };
  socket.onmessage = (event) => {
    const { data } = event as { data: unknown };
    if (typeof data === "string") {
      const control = parseStreamControl(data);
      if (control) handlers.onControl(control);
      return;
    }
    if (data instanceof ArrayBuffer) channel.deliver(data);
  };
  socket.onclose = (event) => {
    channel.onclose?.({ code: event.code, reason: event.reason, wasClean: event.wasClean });
    handlers.onClose({
      code: event.code,
      reason: event.reason,
      authRequired: event.code === AUTH_REQUIRED_CLOSE_CODE,
    });
  };
  socket.onerror = (event) => {
    channel.onerror?.(event);
    handlers.onError?.();
  };
  const sendText = (message: Record<string, unknown>): boolean => {
    if (socket.readyState !== OPEN) return false;
    socket.send(JSON.stringify(message));
    return true;
  };
  return {
    channel,
    sendLease: () => sendText({ type: "lease" }),
    sendResize: (width, height) => sendText({ type: "resize", width, height }),
    close: () => channel.close(1000, "viewer_closed"),
  };
}
