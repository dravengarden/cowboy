import init, { BrowserPeer } from "./cowboy_browser_wireguard_probe.js";

const started = performance.now();
await init();
let peer;
let socket;
let timer;
let lastData;
let damagedOriginal;
let corruptNext = false;
let dropHandshake = false;
let dropped = 0;
let leak = false;
const inbox = [];
let lastReceiver;

function transmit(bytes) {
  if (new TextDecoder().decode(bytes).includes("WGPROBE_PAYLOAD_SECRET_")) {
    leak = true;
  }
  if (bytes[0] === 1 && dropHandshake) {
    dropHandshake = false;
    dropped++;
    return;
  }
  if (bytes[0] === 4 && bytes.length > 32) {
    lastData = bytes.slice();
    lastReceiver = new DataView(
      bytes.buffer,
      bytes.byteOffset,
      bytes.byteLength,
    ).getUint32(4, true);
    if (corruptNext) {
      corruptNext = false;
      damagedOriginal = bytes.slice();
      bytes = bytes.slice();
      bytes[bytes.length - 1] ^= 128;
    }
  }
  if (socket?.readyState !== WebSocket.OPEN) {
    throw new Error("carrier is not open");
  }
  socket.send(bytes);
}
function drain() {
  for (let packet; (packet = peer.network());) transmit(packet);
  for (let payload; (payload = peer.payload());) {
    if (inbox.length >= 256) throw new Error("fixture receive queue full");
    inbox.push(Array.from(payload));
  }
}
function startTimer() {
  clearInterval(timer);
  timer = setInterval(() => {
    peer.tick();
    drain();
  }, 25);
}
async function close() {
  clearInterval(timer);
  if (!socket || socket.readyState === WebSocket.CLOSED) return;
  const old = socket;
  await new Promise((resolve) => {
    old.addEventListener("close", resolve, { once: true });
    old.close();
  });
}
async function connect(url) {
  await close();
  socket = new WebSocket(url);
  socket.binaryType = "arraybuffer";
  socket.addEventListener("message", ({ data }) => {
    peer.receive(new Uint8Array(data));
    drain();
  });
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener(
      "error",
      () => reject(new Error("WSS connection failed")),
      { once: true },
    );
  });
  startTimer();
}
const handlers = {
  async create({ key }) {
    await close();
    peer?.free();
    peer = new BrowserPeer(new Uint8Array(key));
    lastData = damagedOriginal = undefined;
    lastReceiver = undefined;
    inbox.length = 0;
    return { publicKey: Array.from(peer.public_key()) };
  },
  connect: ({ url }) => connect(url),
  send({ payload, forged = false }) {
    peer.send(new Uint8Array(payload), forged);
    drain();
  },
  stats() {
    return {
      ...JSON.parse(peer.stats()),
      dropped,
      leak,
      lastReceiver,
      buffered: socket?.bufferedAmount ?? 0,
    };
  },
  inbox() {
    return inbox.splice(0);
  },
  replay() {
    socket.send(lastData);
  },
  corrupt({ payload }) {
    corruptNext = true;
    peer.send(new Uint8Array(payload), false);
    drain();
  },
  repair() {
    socket.send(damagedOriginal);
  },
  reset() {
    peer.reset();
    inbox.length = 0;
  },
  dropHandshake() {
    dropHandshake = true;
  },
  pause: () => close(),
  async resume({ url }) {
    peer.reset();
    await connect(url);
  },
};
onmessage = async ({ data: { id, op, args = {} } }) => {
  try {
    if (!handlers[op]) throw new Error(`Unknown fixture operation ${op}`);
    const result = await handlers[op](args);
    postMessage({ id, ok: true, result });
  } catch (error) {
    postMessage({
      id,
      ok: false,
      error: `${String(error)}\n${error.stack ?? ""}`,
    });
  }
};
postMessage({
  ready: true,
  startupMs: performance.now() - started,
  worker: typeof document === "undefined",
});
