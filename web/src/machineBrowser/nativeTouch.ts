/** Native multi-touch for the Machine browser viewer
 * (plugins/machine-browser/runtime/native-touch.patch). The viewer asks for
 * pseudo-encoding -1883 in noVNC's SetEncodings; only once the display
 * server answers with message 192 does it send touch contacts as message 193
 * (u8 type, u8 state 0 begin / 1 update / 2 end, u32 id, u16 x, u16 y, big
 * endian, framebuffer pixels), and only then, between touch sequences, does
 * it take touch input away from noVNC's gesture-to-mouse emulation. Without the answer (a display
 * server without the patch or with the feature off) nothing changes: noVNC
 * keeps emulating the mouse and no 193 is ever sent, because a stock server
 * disconnects on it. Mouse and pen input always stay with noVNC.
 *
 * noVNC 1.7 has no extension points for this, so the module hooks its
 * `RFB.messages.clientEncodings` (once, for opted-in sockets only) and the
 * instance's `_normalMsg`; `nativeTouch.test.ts` pins both against fakes and
 * the encoder against the patch's wire layout. Framework-free. */

export const TOUCH_PSEUDO_ENCODING = -1883;
export const TOUCH_ACK_MESSAGE = 192;
export const TOUCH_MESSAGE = 193;
export const TOUCH_BEGIN = 0;
export const TOUCH_UPDATE = 1;
export const TOUCH_END = 2;
/** The server accepts at most this many contacts at once. */
export const MAX_CONTACTS = 10;

export type TouchState = typeof TOUCH_BEGIN | typeof TOUCH_UPDATE | typeof TOUCH_END;

/** The noVNC `Websock` calls this module makes. */
export interface RfbSocket {
  rQpeek8(): number;
  rQshift8(): number;
  rQwait(message: string, bytes: number): boolean;
  sQpushBytes(bytes: Uint8Array): void;
  flush(): void;
}

/** The noVNC `RFB` internals this module reads or wraps. */
export interface RfbInternals {
  _sock: RfbSocket;
  _FBU: { rects: number };
  _rfbConnectionState: string;
  _normalMsg(): boolean;
}

export interface RfbMessages {
  clientEncodings(sock: RfbSocket, encodings: number[]): void;
}

/** One touch message, exactly as the display server reads it. */
export function encodeTouch(state: TouchState, id: number, x: number, y: number): Uint8Array {
  const bytes = new Uint8Array(10);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, TOUCH_MESSAGE);
  view.setUint8(1, state);
  view.setUint32(2, id >>> 0);
  view.setUint16(6, x);
  view.setUint16(8, y);
  return bytes;
}

const requesting = new WeakSet<RfbSocket>();
const hooked = new WeakSet<RfbMessages>();

/** Wrap noVNC's SetEncodings writer so that opted-in sockets also request
 * native touch; every other socket's message is unchanged. Idempotent. */
export function hookEncodings(messages: Partial<RfbMessages> | undefined): void {
  if (
    !messages ||
    typeof messages.clientEncodings !== "function" ||
    hooked.has(messages as RfbMessages)
  ) {
    return;
  }
  hooked.add(messages as RfbMessages);
  const original = messages.clientEncodings.bind(messages);
  messages.clientEncodings = (sock: RfbSocket, encodings: number[]) =>
    original(sock, requesting.has(sock) ? [...encodings, TOUCH_PSEUDO_ENCODING] : encodings);
}

/** Wrap the instance's message loop: consume the acknowledgement (u8 type,
 * u8 version, u8 contacts) before noVNC, which would fail on an unknown
 * server message; leave every other message to noVNC. */
export function hookAcknowledgement(
  rfb: RfbInternals,
  onAck: (version: number, contacts: number) => void,
): void {
  const original = rfb._normalMsg.bind(rfb);
  requesting.add(rfb._sock);
  rfb._normalMsg = () => {
    const sock = rfb._sock;
    if (rfb._FBU.rects > 0 || sock.rQwait("message", 1) || sock.rQpeek8() !== TOUCH_ACK_MESSAGE) {
      return original();
    }
    if (sock.rQwait("TouchEventsSupported", 3)) return false;
    sock.rQshift8();
    const version = sock.rQshift8();
    const contacts = sock.rQshift8();
    onAck(version, contacts);
    return true;
  };
}

/** The pointer event fields used; `PointerEvent` satisfies it. */
export interface TouchPointer {
  pointerId: number;
  pointerType: string;
  clientX: number;
  clientY: number;
  preventDefault(): void;
  stopPropagation(): void;
}

export interface CanvasGeometry {
  /** Framebuffer pixels (noVNC keeps the canvas at the remote size). */
  width: number;
  height: number;
  getBoundingClientRect(): { left: number; top: number; width: number; height: number };
}

/** The framebuffer pixel under a client point, or `undefined` when the
 * point lies outside the scaled framebuffer (the letterbox) and `clamp` is
 * off. noVNC scales the canvas with CSS, so the mapping is the ratio of its
 * pixel size to its laid-out size. */
export function framebufferPoint(
  canvas: CanvasGeometry,
  clientX: number,
  clientY: number,
  clamp: boolean,
): { x: number; y: number } | undefined {
  const rect = canvas.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0 || canvas.width <= 0 || canvas.height <= 0) {
    return undefined;
  }
  const x = Math.floor(((clientX - rect.left) * canvas.width) / rect.width);
  const y = Math.floor(((clientY - rect.top) * canvas.height) / rect.height);
  const inside = x >= 0 && y >= 0 && x < canvas.width && y < canvas.height;
  if (!inside && !clamp) return undefined;
  return {
    x: Math.min(Math.max(x, 0), canvas.width - 1),
    y: Math.min(Math.max(y, 0), canvas.height - 1),
  };
}

export interface NativeTouchOptions {
  /** Receives the viewer's pointer and touch events (capture phase), above
   * noVNC's canvas. */
  surface: EventTarget & {
    setPointerCapture?(pointerId: number): void;
    releasePointerCapture?(pointerId: number): void;
  };
  canvas(): CanvasGeometry | null | undefined;
  /** Where blur and pagehide arrive (the window). */
  page: EventTarget;
  /** Where visibilitychange arrives, with its state. */
  visibility: EventTarget & { visibilityState?: string };
  /** A contact began (the viewer focuses the canvas as noVNC would). */
  onContact?(): void;
}

export interface NativeTouch {
  /** Whether native touch drives touch input: acknowledged by the display
   * server, and no sequence noVNC started is still running. */
  readonly enabled: boolean;
  /** End every active contact (blur, hidden page). */
  cancelAll(): void;
  /** Stop listening; the server ends whatever is still active when the
   * connection closes, so nothing more is sent. */
  close(): void;
}

interface Contact {
  id: number;
  x: number;
  y: number;
}

const POINTER_EVENTS = ["pointerdown", "pointermove", "pointerup", "pointercancel"] as const;
const TOUCH_EVENTS = ["touchstart", "touchmove", "touchend", "touchcancel"] as const;

/** Whether `rfb` still has the noVNC internals this module hooks; a noVNC
 * upgrade that renamed them leaves the viewer on the emulated mouse. */
export function hookable(rfb: Partial<RfbInternals>): rfb is RfbInternals {
  const sock = rfb._sock as Partial<RfbSocket> | undefined;
  return (
    typeof rfb._normalMsg === "function" &&
    typeof rfb._FBU === "object" &&
    typeof rfb._rfbConnectionState === "string" &&
    ["rQpeek8", "rQshift8", "rQwait", "sQpushBytes", "flush"].every(
      (name) => typeof sock?.[name as keyof RfbSocket] === "function",
    )
  );
}

/** Native touch turns on only between touch sequences: a sequence that
 * began while noVNC emulated the mouse must reach noVNC to its end, or its
 * emulated button stays pressed. The gate follows those sequences (touch
 * pointers down, and every Touch identifier noVNC saw start until noVNC saw
 * its own touchend or touchcancel) and, once acknowledged and idle,
 * activates after the current event has been dispatched to noVNC. A touch
 * whose terminal event never arrives keeps the emulated mouse for good:
 * another gesture finishing says nothing about it. */
export function activationGate(
  activate: () => void,
  schedule: (run: () => void) => void = (run) => setTimeout(run, 0),
) {
  let acknowledged = false;
  let scheduled = false;
  const pointers = new Set<number>();
  const touches = new Set<number>();
  const idle = () => pointers.size === 0 && touches.size === 0;
  const check = (): void => {
    if (!acknowledged || scheduled || !idle()) return;
    scheduled = true;
    schedule(() => {
      scheduled = false;
      if (acknowledged && idle()) {
        acknowledged = false;
        activate();
      }
    });
  };
  return {
    acknowledge(): void {
      acknowledged = true;
      check();
    },
    pointer(type: string, pointerId: number): void {
      if (type === "pointerdown") pointers.add(pointerId);
      else if (type === "pointerup" || type === "pointercancel") pointers.delete(pointerId);
      check();
    },
    /** noVNC received this touch event for these changed identifiers. */
    touch(type: string, changed: readonly number[]): void {
      for (const identifier of changed) {
        if (type === "touchstart") touches.add(identifier);
        else if (type === "touchend" || type === "touchcancel") touches.delete(identifier);
      }
      check();
    },
    /** The page lost focus or visibility. Pointer ids are forgotten (their
     * up may never arrive), but every touch noVNC saw start stays open
     * until noVNC sees its own touchend or touchcancel, or its emulated
     * button stays pressed. */
    reset(): void {
      pointers.clear();
      check();
    },
  };
}

/** The identifiers of a touch event's `changedTouches`. */
export function changedIdentifiers(event: Event): number[] {
  const changed = (event as Event & { changedTouches?: ArrayLike<{ identifier: number }> })
    .changedTouches;
  return changed ? Array.from(changed, (touch) => touch.identifier) : [];
}

const DISABLED: NativeTouch = { enabled: false, cancelAll: () => {}, close: () => {} };

export function attachNativeTouch(
  rfb: Partial<RfbInternals>,
  options: NativeTouchOptions,
): NativeTouch {
  if (!hookable(rfb)) return DISABLED;
  let enabled = false;
  let closed = false;
  let nextId = 1;
  const contacts = new Map<number, Contact>();
  const gate = activationGate(() => {
    enabled = !closed;
  });
  hookAcknowledgement(rfb, (version, count) => {
    if (!closed && version >= 1 && count > 0) gate.acknowledge();
  });
  const send = (state: TouchState, contact: Contact): void => {
    if (rfb._rfbConnectionState !== "connected") return;
    rfb._sock.sQpushBytes(encodeTouch(state, contact.id, contact.x, contact.y));
    rfb._sock.flush();
  };
  const end = (pointerId: number): void => {
    const contact = contacts.get(pointerId);
    if (!contact) return;
    contacts.delete(pointerId);
    send(TOUCH_END, contact);
    try {
      options.surface.releasePointerCapture?.(pointerId);
    } catch {
      // the capture is already gone
    }
  };
  const cancelAll = (): void => {
    for (const pointerId of contacts.keys()) end(pointerId);
  };
  const onPointer = (event: Event): void => {
    const pointer = event as unknown as TouchPointer;
    if (pointer.pointerType !== "touch") return;
    if (!enabled) {
      gate.pointer(event.type, pointer.pointerId);
      return;
    }
    pointer.preventDefault();
    pointer.stopPropagation();
    const canvas = options.canvas();
    const existing = contacts.get(pointer.pointerId);
    if (event.type === "pointerdown") {
      if (existing || contacts.size >= MAX_CONTACTS || !canvas) return;
      const point = framebufferPoint(canvas, pointer.clientX, pointer.clientY, false);
      if (!point) return;
      const contact = { id: nextId, ...point };
      nextId = nextId >= 0xffffffff ? 1 : nextId + 1;
      contacts.set(pointer.pointerId, contact);
      try {
        options.surface.setPointerCapture?.(pointer.pointerId);
      } catch {
        // capture is best effort; moves still arrive while inside
      }
      send(TOUCH_BEGIN, contact);
      options.onContact?.();
      return;
    }
    if (!existing) return;
    // A cancelled pointer ends where it last was; its own position may be
    // meaningless.
    const point =
      event.type !== "pointercancel" &&
      canvas &&
      framebufferPoint(canvas, pointer.clientX, pointer.clientY, true);
    if (point) {
      existing.x = point.x;
      existing.y = point.y;
    }
    if (event.type === "pointermove") {
      if (point) send(TOUCH_UPDATE, existing);
    } else {
      end(pointer.pointerId);
    }
  };
  // While native touch is on, noVNC's canvas never sees touch events: its
  // gesture handler would emulate a mouse, and the browser's compatibility
  // mouse events and click follow an unprevented touch.
  const onTouch = (event: Event): void => {
    if (!enabled) {
      gate.touch(event.type, changedIdentifiers(event));
      return;
    }
    if (event.cancelable) event.preventDefault();
    event.stopPropagation();
  };
  const onLeave = (): void => {
    cancelAll();
    gate.reset();
  };
  const onHidden = (): void => {
    if (options.visibility.visibilityState === "hidden") onLeave();
  };
  for (const type of POINTER_EVENTS) {
    options.surface.addEventListener(type, onPointer, { capture: true });
  }
  for (const type of TOUCH_EVENTS) {
    options.surface.addEventListener(type, onTouch, { capture: true, passive: false });
  }
  options.page.addEventListener("blur", onLeave);
  options.page.addEventListener("pagehide", onLeave);
  options.visibility.addEventListener("visibilitychange", onHidden);
  return {
    get enabled() {
      return enabled;
    },
    cancelAll,
    close: () => {
      if (closed) return;
      closed = true;
      enabled = false;
      contacts.clear();
      for (const type of POINTER_EVENTS) {
        options.surface.removeEventListener(type, onPointer, { capture: true });
      }
      for (const type of TOUCH_EVENTS) {
        options.surface.removeEventListener(type, onTouch, { capture: true });
      }
      options.page.removeEventListener("blur", onLeave);
      options.page.removeEventListener("pagehide", onLeave);
      options.visibility.removeEventListener("visibilitychange", onHidden);
    },
  };
}
