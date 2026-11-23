// Connection traces for store.ts in a unit test. The store runs unchanged:
// the product sync database is the real implementation over FakeIndexedDb,
// the device proof is a fixed subprotocol, and WebSocket, location and
// document are minimal stand-ins installed around the import. Tests in this
// file share one store instance and run in order. The module mocks and DOM
// stand-ins would leak into other files in one bun process, so this file is
// not a *.test.ts: `bun run test` and the Nix web build run it on its own.

import { expect, mock, test } from "bun:test";
import * as realDatabase from "./productSyncDatabase.ts";
import * as realDevice from "./browserDevice.ts";
import { FakeIndexedDb } from "./idbPersistence.fixture.ts";

class FakeSocket extends EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static all: FakeSocket[] = [];
  readyState = FakeSocket.CONNECTING;
  protocol = "";
  sent: string[] = [];
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  constructor(public url: string, public protocols: string[]) {
    super();
    FakeSocket.all.push(this);
  }
  send(data: string): void {
    this.sent.push(data);
  }
  close(): void {
    this.readyState = FakeSocket.CLOSED;
  }
  /** The server accepts the upgrade with the product sync subprotocol. */
  accept(): void {
    this.readyState = FakeSocket.OPEN;
    this.protocol = realDatabase.PRODUCT_SYNC_SUBPROTOCOL;
    this.onopen?.(new Event("open"));
  }
  /** The server sends one Outbound frame. */
  receive(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) } as MessageEvent);
  }
  /** The server ends this socket. */
  drop(code = 1006): void {
    this.readyState = FakeSocket.CLOSED;
    this.onclose?.({ code, wasClean: false, reason: "" } as CloseEvent);
  }
}

const dataset: realDatabase.SyncDataset = {
  schema: "dravengarden.cowboy.product-sync-dataset/v1",
  dataset_id: `dataset-${"a".repeat(64)}`,
  user_id: "user-1",
  database_version: 2,
  outbox_contract: "atomic-delta-v1",
};
const database = realDatabase.createProductSyncDatabase(
  () => "user-1",
  async () => dataset,
  { factory: new FakeIndexedDb() },
);
mock.module("./productSyncDatabase.ts", () => ({
  ...realDatabase,
  productSyncDatabase: database,
}));
mock.module("./browserDevice.ts", () => ({
  ...realDevice,
  browserDeviceProtocol: () => Promise.resolve("cowboy-device-test"),
}));
// Virtual timer clock so backoff and capacity-retry delays are verified at
// their exact millisecond boundaries without sleeping 10+ real seconds per run.
const realSetTimeout = globalThis.setTimeout.bind(globalThis);
const realDateNow = Date.now.bind(Date);
let virtualNow = 1_700_000_000_000;
let nextTimerId = 1;
const pendingTimers = new Map<number, { due: number; fn: () => void }>();

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await new Promise<void>((resolve) => {
      realSetTimeout(resolve, 0);
    });
  }
}

async function advanceTime(ms: number): Promise<void> {
  const target = virtualNow + ms;
  await flushMicrotasks();
  while (true) {
    let earliestId: number | undefined;
    let earliestDue = Infinity;
    for (const [id, entry] of pendingTimers) {
      if (entry.due <= target && entry.due < earliestDue) {
        earliestDue = entry.due;
        earliestId = id;
      }
    }
    if (earliestId === undefined) break;
    const entry = pendingTimers.get(earliestId)!;
    pendingTimers.delete(earliestId);
    virtualNow = Math.max(virtualNow, entry.due);
    entry.fn();
    await flushMicrotasks();
  }
  virtualNow = target;
  await flushMicrotasks();
}

const globals = globalThis as Record<string, unknown>;
globals.Date = Object.assign(Date, { now: () => virtualNow });
globals.setTimeout = ((callback: (...args: unknown[]) => void, delay = 0, ...args: unknown[]) => {
  const id = nextTimerId++;
  const due = virtualNow + Math.max(0, Number(delay) || 0);
  pendingTimers.set(id, { due, fn: () => callback(...args) });
  return id as unknown as ReturnType<typeof setTimeout>;
}) as typeof setTimeout;
globals.clearTimeout = ((handle: unknown) => {
  pendingTimers.delete(Number(handle));
}) as typeof clearTimeout;
globals.WebSocket = FakeSocket;
globals.location = { protocol: "https:", host: "cowboy.test" };
// UI modules the store reaches probe the DOM at import; install the document
// only after the import so they take their non-browser paths.
const store = await import("./store.ts");
globals.window = globalThis;
globals.requestAnimationFrame = (callback: FrameRequestCallback): number =>
  globalThis.setTimeout(() => callback(performance.now()), 16) as unknown as number;
globals.cancelAnimationFrame = (handle: number): void => globalThis.clearTimeout(handle);
globals.document = Object.assign(new EventTarget(), {
  visibilityState: "visible",
  hidden: false,
  querySelector: () => null,
  querySelectorAll: () => [],
});

async function until(condition: () => boolean, timeoutMs = 3000): Promise<void> {
  const deadline = realDateNow() + timeoutMs;
  while (!condition()) {
    if (realDateNow() > deadline) throw new Error("condition not reached");
    await new Promise((resolve) => realSetTimeout(resolve, 2));
  }
}
const settle = (ms: number): Promise<void> => advanceTime(ms);

test("a user retry opens one socket bound to the dataset and device proof", async () => {
  store.retrySyncNow();
  await until(() => FakeSocket.all.length === 1);
  const [socket] = FakeSocket.all;
  const url = new URL(socket!.url);
  expect(url.protocol).toBe("wss:");
  expect(url.host).toBe("cowboy.test");
  expect(url.pathname).toBe("/ws");
  expect(url.searchParams.get("dataset")).toBe(dataset.dataset_id);
  expect(url.searchParams.get("bootstrap")).toBe("lazy");
  expect(socket!.protocols).toEqual([realDatabase.PRODUCT_SYNC_SUBPROTOCOL, "cowboy-device-test"]);
  await settle(50);
  expect(FakeSocket.all.length).toBe(1);
});

test("a second retry replaces the socket and the superseded one's close is inert", async () => {
  const first = FakeSocket.all.at(-1)!;
  store.retrySyncNow();
  await until(() => FakeSocket.all.length === 2);
  expect(first.readyState).toBe(FakeSocket.CLOSED);
  // The replaced socket's late close must not schedule another reconnect.
  first.drop();
  await settle(1500);
  expect(FakeSocket.all.length).toBe(2);
});

test("losing the current socket reconnects after the first backoff, not before", async () => {
  const current = FakeSocket.all.at(-1)!;
  const before = FakeSocket.all.length;
  current.drop();
  await settle(700);
  expect(FakeSocket.all.length).toBe(before);
  await settle(3_000);
  await until(() => FakeSocket.all.length === before + 1, 3000);
  expect(new URL(FakeSocket.all.at(-1)!.url).searchParams.get("dataset")).toBe(dataset.dataset_id);
});

test("an open socket is live only after bootstrap completes", async () => {
  const socket = FakeSocket.all.at(-1)!;
  socket.accept();
  expect(store.getSyncStatus().phase).not.toBe("live");
  socket.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
});

test("a focus commit does not reopen the session selected by the click", () => {
  const socket = FakeSocket.all.at(-1)!;
  const first = socket.sent.length;
  store.openSession("focused");
  store.openSession("focused", { reassert: false });
  store.openSession("restored", { reassert: false });
  store.openSession("restored");
  const opens = socket.sent.slice(first).map((data) => JSON.parse(data))
    .filter((frame) => frame.type === "open_session");
  expect(opens.map((frame) => frame.session_id)).toEqual(["focused", "restored", "restored"]);
});

test("capacity admission presents waiting with the queue position", async () => {
  const before = FakeSocket.all.length;
  store.retrySyncNow();
  await until(() => FakeSocket.all.length === before + 1);
  const socket = FakeSocket.all.at(-1)!;
  socket.accept();
  socket.receive({
    type: "client_capacity",
    capacity: { status: "waiting", position: 3, channel_limit: 4 },
  });
  await until(() => store.getSyncStatus().phase === "waiting");
  expect(store.getSyncStatus().position).toBe(3);
  socket.receive({ type: "client_capacity", capacity: { status: "active", channel_limit: 4 } });
  socket.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
});

test("a user retry during backoff clears the pending retry countdown", async () => {
  const live = FakeSocket.all.at(-1)!;
  live.drop();
  await until(() => store.getSyncStatus().retryAt !== undefined);
  const before = FakeSocket.all.length;
  store.retrySyncNow();
  await until(() => FakeSocket.all.length === before + 1);
  // The scheduled retry was cancelled; presenting its countdown would claim a
  // retry that never comes.
  expect(store.getSyncStatus().retryAt).toBeUndefined();
});

test("a capacity close waits for the admission retry delay before reconnecting", async () => {
  const current = FakeSocket.all.at(-1)!;
  current.accept();
  current.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
  const before = FakeSocket.all.length;
  current.drop(4429);
  // No retry_after_ms was announced, so the Controller default of 5 s applies.
  await settle(4_999);
  expect(FakeSocket.all.length).toBe(before);
  await settle(1);
  await until(() => FakeSocket.all.length === before + 1, 4_000);
}, 10_000);

// Last: an auth-required close pauses the product socket until the sign-in
// cookie changes, which this harness never signals.
test("an auth-required close pauses reconnecting until sign-in changes", async () => {
  const current = FakeSocket.all.at(-1)!;
  current.accept();
  current.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
  const before = FakeSocket.all.length;
  current.drop(4001);
  await until(() => store.getSyncStatus().phase === "auth_required");
  await settle(1_500);
  store.retrySyncNow();
  await settle(200);
  expect(FakeSocket.all.length).toBe(before);
  expect(store.getSyncStatus().phase).toBe("auth_required");
});
