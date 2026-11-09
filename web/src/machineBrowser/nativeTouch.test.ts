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
  grantedContacts,
  hookAcknowledgement,
  hookEncodings,
} from "./nativeTouch";

/** noVNC's Websock: a receive queue fed by the test, a send queue whose
 * flushes the test reads back. */
class FakeSock implements RfbSocket {
  received: number[] = [];
  pending: number[] = [];
  flushed: number[][] = [];
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

/** Let the gate's deferred activation run. */
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function setup(acknowledge = true) {
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
    await tick();
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

test("without the acknowledgement touch input stays with noVNC", async () => {
  const { surface, sent } = await setup(false);
  const down = pointer("pointerdown", 1, 100, 100);
  surface.dispatchEvent(down);
  const start = new Event("touchstart", { cancelable: true });
  surface.dispatchEvent(start);
  assertEquals(down.defaultPrevented, false);
  assertEquals(start.defaultPrevented, false);
  assertEquals(sent(), []);
});

test("acknowledged touch sends begin, moves and end in framebuffer pixels", async () => {
  const { surface, sent, begun } = await setup();
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

test("two fingers are two contacts and noVNC never sees their touch events", async () => {
  const { surface, sent } = await setup();
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

test("letterbox, mouse and the eleventh contact are not sent", async () => {
  const { surface, sent } = await setup();
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

test("blur, a hidden page and pagehide end every active contact", async () => {
  const { surface, page, visibility, sent } = await setup();
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

test("a closed or disconnected session sends nothing and gives touch back", async () => {
  const disconnected = await setup();
  disconnected.rfb._rfbConnectionState = "disconnected";
  disconnected.surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  assertEquals(disconnected.sent(), []);

  const { surface, touch, sent } = await setup();
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  touch.close();
  assertEquals(touch.enabled, false);
  surface.dispatchEvent(pointer("pointerup", 1, 100, 100));
  const start = new Event("touchstart", { cancelable: true });
  surface.dispatchEvent(start);
  assertEquals(start.defaultPrevented, false);
  assertEquals(sent().length, 1);
});

test("a noVNC without the hooked internals keeps the emulated mouse", () => {
  const surface = new FakeSurface();
  const touch = attachNativeTouch(
    { _sock: new FakeSock() },
    { surface, canvas: () => CANVAS, page: new EventTarget(), visibility: new EventTarget() },
  );
  const down = pointer("pointerdown", 1, 100, 100);
  surface.dispatchEvent(down);
  assertEquals(touch.enabled, false);
  assertEquals(down.defaultPrevented, false);
  hookEncodings(undefined);
  hookEncodings({});
});

/** A touch event whose `changedTouches` carry these identifiers. */
function touchEvent(type: string, changed: number[]) {
  const event = Object.assign(new Event(type, { cancelable: true }), {
    changedTouches: changed.map((identifier) => ({ identifier })),
    stopped: false,
  });
  event.stopPropagation = () => {
    event.stopped = true;
  };
  return event;
}

test("an acknowledgement during an emulated drag waits for that drag to end", async () => {
  const { rfb, surface, touch, sent } = await setup(false);
  // noVNC is emulating a mouse drag when the acknowledgement arrives.
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  surface.dispatchEvent(touchEvent("touchstart", [1]));
  rfb._sock.received.push(192, 1, 10);
  rfb._normalMsg();
  await tick();
  assertEquals(touch.enabled, false);
  const move = pointer("pointermove", 1, 110, 100);
  const touchMove = touchEvent("touchmove", [1]);
  surface.dispatchEvent(move);
  surface.dispatchEvent(touchMove);
  surface.dispatchEvent(pointer("pointerup", 1, 120, 100));
  await tick();
  // The pointer is up but noVNC has not seen touchend yet.
  assertEquals(touch.enabled, false);
  const touchEnd = touchEvent("touchend", [1]);
  surface.dispatchEvent(touchEnd);
  // Every event of the emulated drag reached noVNC unprevented.
  assertEquals(
    [move, touchMove, touchEnd].map((event) => event.defaultPrevented),
    [false, false, false],
  );
  assertEquals(sent(), []);
  await tick();
  assertEquals(touch.enabled, true);
  surface.dispatchEvent(pointer("pointerdown", 2, 100, 100));
  assertEquals(
    sent().map(({ state }) => state),
    [TOUCH_BEGIN],
  );
});

test("a second finger landing before activation keeps deferring it", async () => {
  const { rfb, surface, page, touch } = await setup(false);
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  surface.dispatchEvent(touchEvent("touchstart", [1]));
  rfb._sock.received.push(192, 1, 10);
  rfb._normalMsg();
  surface.dispatchEvent(pointer("pointerup", 1, 100, 100));
  surface.dispatchEvent(touchEvent("touchend", [1]));
  // Before the deferred activation runs, another touch starts.
  surface.dispatchEvent(pointer("pointerdown", 2, 100, 100));
  surface.dispatchEvent(touchEvent("touchstart", [2]));
  await tick();
  assertEquals(touch.enabled, false);
  // Losing the page forgets the pointers, but touch 2 stays open until
  // noVNC sees its own terminal event.
  page.dispatchEvent(new Event("blur"));
  await tick();
  assertEquals(touch.enabled, false);
  surface.dispatchEvent(touchEvent("touchcancel", [2]));
  await tick();
  assertEquals(touch.enabled, true);
});

/** noVNC's gesture handler as far as the remote button goes: it tracks
 * touches by identifier like noVNC's GestureHandler, holds the button while
 * any is tracked, releases it when the last tracked one ends or cancels, and
 * never sees an event native touch stopped in the capture phase. */
function emulatedButton(surface: EventTarget) {
  const tracked = new Set<number>();
  const state = {
    get pressed() {
      return tracked.size > 0;
    },
  };
  for (const type of ["touchstart", "touchend", "touchcancel"]) {
    surface.addEventListener(type, (event) => {
      const touch = event as ReturnType<typeof touchEvent>;
      if (touch.stopped) return;
      for (const { identifier } of touch.changedTouches) {
        if (type === "touchstart") tracked.add(identifier);
        else tracked.delete(identifier);
      }
    });
  }
  return state;
}

test("blur during an emulated drag still lets its delayed touchend release the button", async () => {
  const { rfb, surface, page, touch, sent } = await setup(false);
  const button = emulatedButton(surface);
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  surface.dispatchEvent(touchEvent("touchstart", [1]));
  assertEquals(button.pressed, true);
  // The acknowledgement arrives mid-drag, then the page loses focus.
  rfb._sock.received.push(192, 1, 10);
  rfb._normalMsg();
  page.dispatchEvent(new Event("blur"));
  await tick();
  assertEquals(touch.enabled, false);
  surface.dispatchEvent(touchEvent("touchmove", [1]));
  const end = touchEvent("touchend", [1]);
  surface.dispatchEvent(end);
  assertEquals([end.defaultPrevented, end.stopped], [false, false]);
  assertEquals(button.pressed, false);
  assertEquals(sent(), []);
  // Only now does native touch take over, and noVNC sees nothing more.
  await tick();
  assertEquals(touch.enabled, true);
  surface.dispatchEvent(touchEvent("touchstart", [2]));
  assertEquals(button.pressed, false);
});

test("a hidden page during an emulated drag defers activation the same way", async () => {
  const { rfb, surface, visibility, touch } = await setup(false);
  const button = emulatedButton(surface);
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  surface.dispatchEvent(touchEvent("touchstart", [1]));
  rfb._sock.received.push(192, 1, 10);
  rfb._normalMsg();
  visibility.visibilityState = "hidden";
  visibility.dispatchEvent(new Event("visibilitychange"));
  await tick();
  assertEquals(touch.enabled, false);
  const cancel = touchEvent("touchcancel", [1]);
  surface.dispatchEvent(cancel);
  assertEquals(cancel.stopped, false);
  assertEquals(button.pressed, false);
  await tick();
  assertEquals(touch.enabled, true);
});

test("a lost terminal event keeps the emulated mouse even after another gesture", async () => {
  const { rfb, surface, page, touch } = await setup(false);
  const button = emulatedButton(surface);
  surface.dispatchEvent(pointer("pointerdown", 1, 100, 100));
  surface.dispatchEvent(touchEvent("touchstart", [1]));
  rfb._sock.received.push(192, 1, 10);
  rfb._normalMsg();
  // The page loses focus and touch 1's terminal event is lost there.
  page.dispatchEvent(new Event("blur"));
  // Back on the page, a different gesture runs to completion.
  surface.dispatchEvent(pointer("pointerdown", 2, 120, 100));
  surface.dispatchEvent(touchEvent("touchstart", [2]));
  surface.dispatchEvent(pointer("pointerup", 2, 120, 100));
  surface.dispatchEvent(touchEvent("touchend", [2]));
  await tick();
  // Touch 1 never terminated: native touch stays off, so its late terminal
  // event still reaches noVNC and releases the button.
  assertEquals(touch.enabled, false);
  assertEquals(button.pressed, true);
  const late = touchEvent("touchcancel", [1]);
  surface.dispatchEvent(late);
  assertEquals(late.stopped, false);
  assertEquals(button.pressed, false);
  await tick();
  assertEquals(touch.enabled, true);
});

/** A file of the pinned noVNC package (`web/package.json`), resolved as
 * Vite resolves it. */
function novncCore(file: string): string {
  return `${Bun.resolveSync("@novnc/novnc", import.meta.dir).replace(/rfb\.js$/, "")}${file}`;
}

test("the real noVNC Websock carries the acknowledgement and touch messages", async () => {
  // noVNC's logging module reads `window.console` once when it loads.
  const scope = globalThis as { window?: unknown };
  const hadWindow = "window" in scope;
  scope.window ??= globalThis;
  const { default: Websock } = await import(novncCore("websock.js"));
  if (!hadWindow) delete scope.window;
  const frames: number[][] = [];
  const channel = {
    binaryType: "arraybuffer",
    protocol: "",
    readyState: 1,
    onopen: null,
    onmessage: null as ((event: { data: ArrayBuffer }) => void) | null,
    onclose: null,
    onerror: null,
    send: (data: Uint8Array) => frames.push([...data]),
    close: () => {},
  };
  const sock = new Websock();
  sock.attach(channel);
  const handled: number[] = [];
  const rfb = {
    _sock: sock,
    _FBU: { rects: 0 },
    _rfbConnectionState: "connected",
    _normalMsg(): boolean {
      handled.push(sock.rQshift8());
      return true;
    },
  };
  const surface = new FakeSurface();
  const touch = attachNativeTouch(rfb, {
    surface,
    canvas: () => CANVAS,
    page: new EventTarget(),
    visibility: new EventTarget(),
  });
  // A bell, then the acknowledgement split across two frames.
  channel.onmessage?.({ data: Uint8Array.from([2, 192, 1]).buffer });
  assertEquals(rfb._normalMsg(), true);
  assertEquals(rfb._normalMsg(), false);
  channel.onmessage?.({ data: Uint8Array.from([10]).buffer });
  assertEquals(rfb._normalMsg(), true);
  assertEquals(handled, [2]);
  await tick();
  assertEquals(touch.enabled, true);
  surface.dispatchEvent(pointer("pointerdown", 1, 110, 120));
  assertEquals(frames, [[193, 0, 0, 0, 0, 1, 0, 200, 0, 200]]);
});

test("the pinned noVNC still has the internals native touch hooks", async () => {
  const source = await Bun.file(novncCore("rfb.js")).text();
  // _sendEncodings writes through RFB.messages.clientEncodings on its socket.
  assertEquals(source.includes("RFB.messages.clientEncodings(this._sock, encs);"), true);
  assertEquals(/\n {4}clientEncodings\(sock, encodings\) \{/.test(source), true);
  // The message loop calls the instance's _normalMsg; FBU rectangles are
  // counted in _FBU.rects; the connection state and socket fields exist.
  assertEquals(source.includes("if (!this._normalMsg()) {"), true);
  assertEquals(source.includes("if (this._FBU.rects > 0) {"), true);
  assertEquals(source.includes("this._rfbConnectionState = state;"), true);
  assertEquals(source.includes("this._sock = new Websock();"), true);
});

test("the acknowledgement's contact count bounds the connection", async () => {
  assertEquals(
    [grantedContacts(1, 0), grantedContacts(0, 10), grantedContacts(1, 3), grantedContacts(1, 200)],
    [undefined, undefined, 3, MAX_CONTACTS],
  );
  const { rfb, surface, touch, sent } = await setup(false);
  rfb._sock.received.push(192, 1, 2);
  rfb._normalMsg();
  await tick();
  assertEquals(touch.enabled, true);
  for (const id of [1, 2, 3]) surface.dispatchEvent(pointer("pointerdown", id, 100 + id, 100));
  // The third finger sends nothing, now or when it moves or lifts.
  const extra = pointer("pointermove", 3, 150, 100);
  surface.dispatchEvent(extra);
  surface.dispatchEvent(pointer("pointerup", 3, 150, 100));
  assertEquals(extra.defaultPrevented, true);
  assertEquals(
    sent().map(({ state, id }) => [state, id]),
    [
      [TOUCH_BEGIN, 1],
      [TOUCH_BEGIN, 2],
    ],
  );
  // Ending one frees its place for the next finger, and nothing stays down.
  surface.dispatchEvent(pointer("pointerup", 1, 101, 100));
  surface.dispatchEvent(pointer("pointerdown", 4, 120, 100));
  surface.dispatchEvent(pointer("pointerup", 4, 120, 100));
  surface.dispatchEvent(pointer("pointerup", 2, 102, 100));
  const states = sent().map(({ state, id }) => [state, id]);
  assertEquals(states.slice(2), [
    [TOUCH_END, 1],
    [TOUCH_BEGIN, 3],
    [TOUCH_END, 3],
    [TOUCH_END, 2],
  ]);
});

test("an acknowledgement granting no contacts keeps the emulated mouse", async () => {
  const { rfb, surface, touch, sent } = await setup(false);
  rfb._sock.received.push(192, 1, 0);
  rfb._normalMsg();
  await tick();
  assertEquals(touch.enabled, false);
  const down = pointer("pointerdown", 1, 100, 100);
  surface.dispatchEvent(down);
  assertEquals([down.defaultPrevented, sent()], [false, []]);
});
