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
const globals = globalThis as Record<string, unknown>;
globals.WebSocket = FakeSocket;
globals.location = { protocol: "https:", host: "cowboy.test" };
// UI modules the store reaches probe the DOM at import; install the document
// only after the import so they take their non-browser paths.
const store = await import("./store.ts");
globals.requestAnimationFrame = (callback: FrameRequestCallback): number =>
  setTimeout(() => callback(performance.now()), 16) as unknown as number;
globals.cancelAnimationFrame = (handle: number): void => clearTimeout(handle);
globals.document = Object.assign(new EventTarget(), {
  visibilityState: "visible",
  hidden: false,
  querySelector: () => null,
  querySelectorAll: () => [],
});

async function until(condition: () => boolean, timeoutMs = 3000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error("condition not reached");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}
const settle = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

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
  await until(() => FakeSocket.all.length === before + 1, 3000);
  expect(new URL(FakeSocket.all.at(-1)!.url).searchParams.get("dataset")).toBe(dataset.dataset_id);
});
