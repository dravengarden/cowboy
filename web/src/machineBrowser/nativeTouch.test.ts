import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  MAX_CONTACTS,
  TOUCH_BEGIN,
  TOUCH_END,
  TOUCH_PSEUDO_ENCODING,
  TOUCH_UPDATE,
  type RfbInternals,
  type RfbMessages,
  type RfbSocket,
  attachNativeTouch,
  encodeTouch,
  framebufferPoint,
  hookAcknowledgement,
  hookEncodings,
} from "./nativeTouch";

/** noVNC's Websock: a receive queue fed by the test, a send queue whose
 * flushes the test reads back. */
class FakeSock implements RfbSocket {
  received: number[] = [];
  pending: number[] = [];
  flushed: number[][] = [];
  rQlen(): number {
    return this.received.length;
  }
  rQpeek8(): number {
    return this.received[0];
  }
  rQshift8(): number {
    return this.received.shift()!;
  }
  rQwait(_message: string, bytes: number): boolean {
    return this.received.length < bytes;
  }
  sQpushBytes(bytes: Uint8Array): void {
    this.pending.push(...bytes);
  }
  flush(): void {
    this.flushed.push(this.pending);
    this.pending = [];
  }
}

class FakeRfb implements RfbInternals {
  _sock = new FakeSock();
  _FBU = { rects: 0 };
  _rfbConnectionState = "connected";
  /** Messages noVNC itself handled. */
  handled: number[] = [];
  _normalMsg(): boolean {
    this.handled.push(this._sock.rQshift8());
    return true;
  }
}

/** A frame whose 720x1280 framebuffer is laid out at 360x640 CSS px from
 * (10, 20): half scale. */
const CANVAS = {
  width: 720,
  height: 1280,
  getBoundingClientRect: () => ({ left: 10, top: 20, width: 360, height: 640 }),
};

class FakeSurface extends EventTarget {
  captured = new Set<number>();
  setPointerCapture(id: number): void {
    this.captured.add(id);
  }
  releasePointerCapture(id: number): void {
    this.captured.delete(id);
  }
}

function pointer(
  type: string,
  pointerId: number,
  clientX: number,
  clientY: number,
  pointerType = "touch",
): Event {
  return Object.assign(new Event(type, { cancelable: true, bubbles: true }), {
    pointerId,
    pointerType,
    clientX,
    clientY,
  });
}

function setup(acknowledge = true) {
  const rfb = new FakeRfb();
  const surface = new FakeSurface();
  const page = new EventTarget();
  const visibility = Object.assign(new EventTarget(), { visibilityState: "visible" });
  let contactsBegun = 0;
  const touch = attachNativeTouch(rfb, {
    surface,
    canvas: () => CANVAS,
    page,
    visibility,
    onContact: () => {
      contactsBegun += 1;
    },
  });
  if (acknowledge) {
    rfb._sock.received.push(192, 1, 10);
    rfb._normalMsg();
  }
  const sent = () => rfb._sock.flushed.map((frame) => decode(frame));
  return { rfb, surface, page, visibility, touch, sent, begun: () => contactsBegun };
}

function decode(frame: number[]) {
  const view = new DataView(Uint8Array.from(frame).buffer);
  return {
    type: view.getUint8(0),
    state: view.getUint8(1),
    id: view.getUint32(2),
    x: view.getUint16(6),
    y: view.getUint16(8),
  };
}

test("touch messages use the patch's wire layout", () => {
  assertEquals(
    [...encodeTouch(TOUCH_UPDATE, 0x01020304, 0x0506, 0x0708)],
    [193, 1, 1, 2, 3, 4, 5, 6, 7, 8],
  );
  assertEquals(
    encodeTouch(TOUCH_END, -1, 0, 0).subarray(2, 6),
    Uint8Array.from([255, 255, 255, 255]),
  );
});

test("only opted-in sockets request native touch, and hooking twice is harmless", () => {
  const calls: number[][] = [];
  const messages: RfbMessages = { clientEncodings: (_sock, encodings) => calls.push(encodings) };
  hookEncodings(messages);
  hookEncodings(messages);
  const plain = new FakeSock();
  const opted = new FakeRfb();
  hookAcknowledgement(opted, () => {});
  messages.clientEncodings(plain, [16, -223]);
  messages.clientEncodings(opted._sock, [16, -223]);
  assertEquals(calls, [
    [16, -223],
    [16, -223, TOUCH_PSEUDO_ENCODING],
  ]);
});

test("the acknowledgement is consumed before noVNC, every other message is noVNC's", () => {
  const rfb = new FakeRfb();
  const acks: Array<[number, number]> = [];
  hookAcknowledgement(rfb, (version, contacts) => acks.push([version, contacts]));
  rfb._sock.received.push(2);
  assertEquals(rfb._normalMsg(), true);
  assertEquals(rfb.handled, [2]);
  // A split acknowledgement waits for its remaining bytes.
  rfb._sock.received.push(192, 1);
  assertEquals(rfb._normalMsg(), false);
  assertEquals(acks, []);
  rfb._sock.received.push(10);
  assertEquals(rfb._normalMsg(), true);
  assertEquals(acks, [[1, 10]]);
  assertEquals(rfb._sock.received, []);
  // Inside a framebuffer update the byte is rectangle data, not a message.
  rfb._FBU.rects = 1;
  rfb._sock.received.push(192);
  rfb._normalMsg();
  assertEquals(rfb.handled, [2, 192]);
});

test("framebuffer points follow noVNC's scaling and the letterbox", () => {
  assertEquals(framebufferPoint(CANVAS, 10, 20, false), { x: 0, y: 0 });
  assertEquals(framebufferPoint(CANVAS, 190, 340, false), { x: 360, y: 640 });
  assertEquals(framebufferPoint(CANVAS, 5, 340, false), undefined);
  assertEquals(framebufferPoint(CANVAS, 5, 700, true), { x: 0, y: 1279 });
});

test("without the acknowledgement touch input stays with noVNC", () => {
  const { surface, sent } = setup(false);
  const down = pointer("pointerdown", 1, 100, 100);
  surface.dispatchEvent(down);
  const start = new Event("touchstart", { cancelable: true });
  surface.dispatchEvent(start);
  assertEquals(down.defaultPrevented, false);
  assertEquals(start.defaultPrevented, false);
  assertEquals(sent(), []);
});

test("acknowledged touch sends begin, moves and end in framebuffer pixels", () => {
  const { surface, sent, begun } = setup();
  const down = pointer("pointerdown", 7, 110, 120);
  surface.dispatchEvent(down);
  surface.dispatchEvent(pointer("pointermove", 7, 120, 120));
  surface.dispatchEvent(pointer("pointerup", 7, 130, 120));
  assertEquals(down.defaultPrevented, true);
  assertEquals(begun(), 1);
  assertEquals(sent(), [
    { type: 193, state: TOUCH_BEGIN, id: 1, x: 200, y: 200 },
    { type: 193, state: TOUCH_UPDATE, id: 1, x: 220, y: 200 },
    { type: 193, state: TOUCH_END, id: 1, x: 240, y: 200 },
  ]);
  assertEquals(surface.captured.size, 0);
});

test("two fingers are two contacts and noVNC never sees their touch events", () => {
  const { surface, sent } = setup();
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 300));
  surface.dispatchEvent(pointer("pointerdown", 2, 200, 300));
  surface.dispatchEvent(pointer("pointermove", 1, 90, 300));
  surface.dispatchEvent(pointer("pointermove", 2, 210, 300));
  let stopped = false;
  const start = Object.assign(new Event("touchstart", { cancelable: true }), {
    stopPropagation: () => {
      stopped = true;
    },
  });
  surface.dispatchEvent(start);
  surface.dispatchEvent(pointer("pointerup", 1, 90, 300));
  surface.dispatchEvent(pointer("pointercancel", 2, 0, 0));
  // Prevented (no compatibility mouse events or click) and stopped in the
  // capture phase, so noVNC's gesture handler on the canvas never runs.
  assertEquals(start.defaultPrevented, true);
  assertEquals(stopped, true);
  assertEquals(
    sent().map(({ state, id, x }) => [state, id, x]),
    [
      [TOUCH_BEGIN, 1, 180],
      [TOUCH_BEGIN, 2, 380],
      [TOUCH_UPDATE, 1, 160],
      [TOUCH_UPDATE, 2, 400],
      [TOUCH_END, 1, 160],
      // A cancelled pointer ends where it last was.
      [TOUCH_END, 2, 400],
    ],
  );
});

test("letterbox, mouse and the eleventh contact are not sent", () => {
  const { surface, sent } = setup();
  surface.dispatchEvent(pointer("pointerdown", 1, 2, 300));
  const mouse = pointer("pointerdown", 2, 100, 100, "mouse");
  surface.dispatchEvent(mouse);
  assertEquals(mouse.defaultPrevented, false);
  for (let index = 0; index < MAX_CONTACTS + 1; index += 1) {
    surface.dispatchEvent(pointer("pointerdown", 10 + index, 100 + index, 100));
  }
  assertEquals(sent().length, MAX_CONTACTS);
  surface.dispatchEvent(pointer("pointermove", 1, 50, 50));
  assertEquals(sent().length, MAX_CONTACTS);
});

test("blur, a hidden page and pagehide end every active contact", () => {
  const { surface, page, visibility, sent } = setup();
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  surface.dispatchEvent(pointer("pointerdown", 2, 150, 100));
  page.dispatchEvent(new Event("blur"));
  assertEquals(
    sent()
      .slice(2)
      .map(({ state, id }) => [state, id]),
    [
      [TOUCH_END, 1],
      [TOUCH_END, 2],
    ],
  );
  surface.dispatchEvent(pointer("pointerdown", 3, 100, 100));
  visibility.visibilityState = "hidden";
  visibility.dispatchEvent(new Event("visibilitychange"));
  surface.dispatchEvent(pointer("pointerdown", 4, 100, 100));
  page.dispatchEvent(new Event("pagehide"));
  assertEquals(
    sent()
      .slice(4)
      .map(({ state, id }) => [state, id]),
    [
      [TOUCH_BEGIN, 3],
      [TOUCH_END, 3],
      [TOUCH_BEGIN, 4],
      [TOUCH_END, 4],
    ],
  );
  // Later moves of an ended pointer send nothing.
  surface.dispatchEvent(pointer("pointermove", 1, 120, 100));
  assertEquals(sent().length, 8);
});

test("a closed or disconnected session sends nothing and gives touch back", () => {
  const disconnected = setup();
  disconnected.rfb._rfbConnectionState = "disconnected";
  disconnected.surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  assertEquals(disconnected.sent(), []);

  const { surface, touch, sent } = setup();
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  touch.close();
  assertEquals(touch.enabled, false);
  surface.dispatchEvent(pointer("pointerup", 1, 100, 100));
  const start = new Event("touchstart", { cancelable: true });
  surface.dispatchEvent(start);
  assertEquals(start.defaultPrevented, false);
  assertEquals(sent().length, 1);
});
