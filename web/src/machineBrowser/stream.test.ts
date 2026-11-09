import { test } from "bun:test";
import { assert, assertEquals } from "../../../tools/test-assert.ts";
import {
  AUTH_REQUIRED_CLOSE_CODE,
  type StreamClose,
  type StreamControl,
  type StreamSocket,
  createBrowserStream,
  parseStreamControl,
} from "./stream";

/** What noVNC's `Websock.attach` requires of a raw channel. */
const RAW_CHANNEL_PROPS = [
  "send",
  "close",
  "binaryType",
  "onerror",
  "onmessage",
  "onopen",
  "protocol",
  "readyState",
];

class FakeSocket implements StreamSocket {
  binaryType: BinaryType = "blob";
  protocol = "";
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  sent: (string | BufferSource)[] = [];
  closed: { code: number | undefined; reason: string | undefined }[] = [];
  send(data: string | BufferSource): void {
    this.sent.push(data);
  }
  close(code?: number, reason?: string): void {
    this.closed.push({ code, reason });
    this.readyState = 3;
  }
  accept(): void {
    this.readyState = 1;
    this.protocol = "cowboy-browser-v1";
    this.onopen?.(new Event("open"));
  }
  text(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) } as MessageEvent);
  }
  binary(bytes: number[]): void {
    this.onmessage?.({ data: Uint8Array.from(bytes).buffer } as MessageEvent);
  }
  drop(code: number, reason = ""): void {
    this.readyState = 3;
    this.onclose?.({ code, reason, wasClean: code === 1000 } as CloseEvent);
  }
}

const state = {
  instance: {
    account_id: "11111111-2222-4333-8444-555555555555",
    instance_id: "b-0123456789abcdef01234567",
    incarnation: 1,
  },
  state: "attached",
  attachment_id: "att-1",
  observed_at_ms: 1,
};

function harness(): {
  socket: FakeSocket;
  stream: ReturnType<typeof createBrowserStream>;
  controls: StreamControl[];
  closes: StreamClose[];
  received: number[][];
  attachNoVnc: () => void;
} {
  const socket = new FakeSocket();
  const controls: StreamControl[] = [];
  const closes: StreamClose[] = [];
  const received: number[][] = [];
  const stream = createBrowserStream(socket, {
    onControl: (message) => controls.push(message),
    onClose: (close) => closes.push(close),
  });
  // What noVNC does in `Websock.attach`.
  const attachNoVnc = (): void => {
    stream.channel.binaryType = "arraybuffer";
    stream.channel.onmessage = (event) => received.push([...new Uint8Array(event.data)]);
    stream.channel.onopen = () => undefined;
    stream.channel.onclose = () => undefined;
    stream.channel.onerror = () => undefined;
  };
  return { socket, stream, controls, closes, received, attachNoVnc };
}

const microtasks = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

test("the channel offers exactly the surface noVNC checks and mirrors the socket", () => {
  const { socket, stream } = harness();
  const { channel } = stream;
  const names = [
    ...Object.keys(channel),
    ...Object.getOwnPropertyNames(Object.getPrototypeOf(channel)),
  ];
  for (const prop of RAW_CHANNEL_PROPS) assert(names.includes(prop), prop);
  assertEquals(socket.binaryType, "arraybuffer");
  assertEquals(channel.readyState, 0);
  socket.accept();
  assertEquals(channel.readyState, 1);
  assertEquals(channel.protocol, "cowboy-browser-v1");
});

test("binary frames reach noVNC and text frames reach the viewer, never the reverse", async () => {
  const { socket, controls, received, attachNoVnc } = harness();
  socket.accept();
  attachNoVnc();
  socket.text({ type: "state", state });
  socket.binary([82, 70, 66]);
  socket.text({ type: "error", code: "resize_failed" });
  socket.text({ type: "auth_session", session: { valid: true } });
  socket.text({ type: "unknown_future" });
  socket.text("not json");
  await microtasks();
  assertEquals(received, [[82, 70, 66]]);
  assertEquals(controls, [
    { type: "state", state },
    { type: "error", code: "resize_failed" },
    { type: "auth_session", session: { valid: true } },
  ]);
});

test("binary frames that arrive before noVNC attaches are delivered in order afterwards", async () => {
  const { socket, received, attachNoVnc } = harness();
  socket.accept();
  socket.binary([1]);
  socket.binary([2]);
  attachNoVnc();
  assertEquals(received, []);
  await microtasks();
  assertEquals(received, [[1], [2]]);
  socket.binary([3]);
  assertEquals(received, [[1], [2], [3]]);
});

test("the channel forwards noVNC's sends and closes while open", () => {
  const { socket, stream, attachNoVnc } = harness();
  attachNoVnc();
  stream.channel.send(Uint8Array.from([9]));
  assertEquals(socket.sent.length, 0, "nothing is sent before the socket opens");
  socket.accept();
  stream.channel.send(Uint8Array.from([9]));
  assertEquals(socket.sent.length, 1);
  assertEquals(stream.sendLease(), true);
  assertEquals(stream.sendResize(780, 1400), true);
  assertEquals(socket.sent.slice(1), [
    JSON.stringify({ type: "lease" }),
    JSON.stringify({ type: "resize", width: 780, height: 1400 }),
  ]);
  stream.close();
  assertEquals(socket.closed, [{ code: 1000, reason: "viewer_closed" }]);
  assertEquals(stream.sendLease(), false);
});

test("close codes propagate to both noVNC and the viewer; 4001 means sign in again", () => {
  const first = harness();
  let noVncClose: { code: number; reason: string; wasClean: boolean } | undefined;
  first.socket.accept();
  first.attachNoVnc();
  first.stream.channel.onclose = (event) => {
    noVncClose = event;
  };
  first.socket.drop(1006, "machine_closed");
  assertEquals(noVncClose, { code: 1006, reason: "machine_closed", wasClean: false });
  assertEquals(first.closes, [{ code: 1006, reason: "machine_closed", authRequired: false }]);
  const second = harness();
  second.socket.accept();
  second.socket.drop(AUTH_REQUIRED_CLOSE_CODE, "auth_required");
  assertEquals(second.closes, [{ code: 4001, reason: "auth_required", authRequired: true }]);
});

test("control parsing accepts only the Controller's message shapes", () => {
  assertEquals(parseStreamControl(JSON.stringify({ type: "state", state })), {
    type: "state",
    state,
  });
  assertEquals(
    parseStreamControl(JSON.stringify({ type: "state", state: { state: "attached" } })),
    undefined,
  );
  assertEquals(parseStreamControl(JSON.stringify({ type: "error", code: "" })), undefined);
  assertEquals(parseStreamControl(JSON.stringify({ type: "error", code: "x" })), {
    type: "error",
    code: "x",
  });
  assertEquals(parseStreamControl("[]"), undefined);
  assertEquals(parseStreamControl("{"), undefined);
});

test("the viewer learns when the socket opens, when a resize can first go out", () => {
  const socket = new FakeSocket();
  const resized: boolean[] = [];
  const stream = createBrowserStream(socket, {
    onOpen: () => resized.push(stream.sendResize(1920, 1080)),
    onControl: () => {},
    onClose: () => {},
  });
  assertEquals(stream.sendResize(1920, 1080), false, "a resize before open is refused");
  socket.accept();
  assertEquals(resized, [true]);
  assertEquals(socket.sent, [JSON.stringify({ type: "resize", width: 1920, height: 1080 })]);
});
