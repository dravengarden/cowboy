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
  expect(url.searchParams.get("session_deltas")).toBe("1");
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

test("active queue edit holds re-declare after socket reconnect until cleared", async () => {
  const first = FakeSocket.all.at(-1)!;
  first.accept();
  first.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
  store.setQueueEditing("s-hold", "q-42");
  const initialFrames = first.sent.map((raw) => JSON.parse(raw))
    .filter((f) => f.type === "set_queue_editing" && f.session_id === "s-hold");
  expect(initialFrames).toEqual([
    expect.objectContaining({ type: "set_queue_editing", session_id: "s-hold", id: "q-42", token: expect.any(String) }),
  ]);

  // Drop and reconnect: the store must automatically re-declare the active hold.
  const before = FakeSocket.all.length;
  first.drop();
  await settle(3_000);
  await until(() => FakeSocket.all.length === before + 1, 4_000);
  const reconnected = FakeSocket.all.at(-1)!;
  reconnected.accept();
  reconnected.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
  const replayedFrames = reconnected.sent.map((raw) => JSON.parse(raw))
    .filter((f) => f.type === "set_queue_editing" && f.session_id === "s-hold");
  expect(replayedFrames).toEqual([
    expect.objectContaining({ type: "set_queue_editing", session_id: "s-hold", id: "q-42", token: expect.any(String) }),
  ]);
  expect(replayedFrames[0].token).not.toBe(initialFrames[0].token);

  // Clearing the hold stops future replays.
  store.setQueueEditing("s-hold", null);
  const beforeClear = FakeSocket.all.length;
  reconnected.drop();
  await settle(3_000);
  await until(() => FakeSocket.all.length === beforeClear + 1, 4_000);
  const afterClear = FakeSocket.all.at(-1)!;
  afterClear.accept();
  afterClear.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
  const clearedFrames = afterClear.sent.map((raw) => JSON.parse(raw))
    .filter((f) => f.type === "set_queue_editing" && f.session_id === "s-hold");
  expect(clearedFrames).toEqual([]);
}, 15_000);

test("session_updated and session_removed incrementally update live store sessions", async () => {
  const socket = FakeSocket.all.at(-1)!;
  socket.accept();
  const base = {
    provider: "codex",
    machine_id: "hawk",
    cwd: "/tmp",
    origin: "web",
    paused: false,
    context_used: 0,
    context_size: 0,
  };
  socket.receive({
    type: "sessions",
    sessions: [
      { ...base, id: "delta-1", title: "First", status: "running" },
      { ...base, id: "delta-2", title: "Second", status: "running" },
    ],
  });
  socket.receive({ type: "bootstrap_complete" });
  await until(() => store.getSyncStatus().phase === "live");
  await settle(25);
  expect(store.currentSessions().map((s) => `${s.id}:${s.status}`)).toEqual([
    "delta-1:running",
    "delta-2:running",
  ]);

  socket.receive({
    type: "session_updated",
    session: { ...base, id: "delta-1", title: "First", status: "busy", context_used: 4200, context_size: 200000 },
  });
  socket.receive({
    type: "session_updated",
    session: { ...base, id: "delta-3", title: "Third", status: "starting" },
  });
  await settle(25);
  expect(store.currentSessions().map((s) => `${s.id}:${s.status}:${s.context_used}`)).toEqual([
    "delta-1:busy:4200",
    "delta-2:running:0",
    "delta-3:starting:0",
  ]);

  const records = new Map<string, string>();
  const fakeStorage: Storage = {
    get length() {
      return records.size;
    },
    key: (index: number) => [...records.keys()][index] ?? null,
    getItem: (key: string) => records.get(key) ?? null,
    setItem: (key: string, value: string) => {
      records.set(key, value);
    },
    removeItem: (key: string) => {
      records.delete(key);
    },
    clear: () => {
      records.clear();
    },
  };
  const previousStorage = globals.localStorage;
  globals.localStorage = fakeStorage;
  try {
    fakeStorage.setItem(
      "cowboy:composer-draft:delta-2",
      JSON.stringify({ text: "bye", attachments: [] }),
    );
    fakeStorage.setItem(
      "cowboy:composer-draft:other-tab-unseen",
      JSON.stringify({ text: "keep me", attachments: [] }),
    );
    const goneEditKey = `cowboy:pending-edit:${JSON.stringify(["delta-2", "draft", "row-1"])}`;
    const keptEditKey = `cowboy:pending-edit:${JSON.stringify(["other-tab-unseen", "draft", "row-2"])}`;
    fakeStorage.setItem(goneEditKey, JSON.stringify({ text: "drop" }));
    fakeStorage.setItem(keptEditKey, JSON.stringify({ text: "preserve" }));

    socket.receive({ type: "session_removed", session_id: "delta-2" });
    await settle(25);
    expect(store.currentSessions().map((s) => s.id)).toEqual(["delta-1", "delta-3"]);
    // Individual removal deletes only the removed session's draft and pending
    // edits, never another tab's buffers for a session this tab has not seen.
    expect(fakeStorage.getItem("cowboy:composer-draft:delta-2")).toBeNull();
    expect(fakeStorage.getItem("cowboy:composer-draft:other-tab-unseen")).not.toBeNull();
    expect(fakeStorage.getItem(goneEditKey)).toBeNull();
    expect(fakeStorage.getItem(keptEditKey)).not.toBeNull();
  } finally {
    globals.localStorage = previousStorage;
  }
});

test("snapshot watermark replays live absorbed chunks without overwriting or duplicating", async () => {
  const socket = FakeSocket.all.at(-1)!;
  const sessionId = "delta-replica-replay";
  // Seed a stale replica tail in IndexedDB before opening the session.
  await database.cache({ kind: "session", session: sessionId, state: "tail" }).save({
    receivedAt: Date.now() - 60_000,
    lastSeq: 2,
    reachedStart: false,
    events: [
      {
        session_id: sessionId,
        seq: 1,
        kind: "update",
        update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "stale" } },
      },
    ],
  });
  store.openSession(sessionId);
  await settle(25);
  // While the stale replica is painted, live WS delivers seq 100 and absorbs seq 101 into it.
  socket.receive({
    type: "event",
    envelope: {
      session_id: sessionId,
      seq: 100,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "hello" } },
    },
  });
  socket.receive({
    type: "event",
    envelope: {
      session_id: sessionId,
      seq: 101,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: " world" } },
    },
  });
  await settle(25);
  // Snapshot captured right after seq 100 (next_seq = 101) arrives: stale replica
  // is discarded, snapshot row 100 ("hello") lands, and buffered seq 101 replays.
  socket.receive({
    type: "snapshot",
    session_id: sessionId,
    reached_start: false,
    next_seq: 101,
    events: [
      {
        session_id: sessionId,
        seq: 100,
        kind: "update",
        update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "hello" } },
      },
    ],
  });
  await settle(25);
  const timeline = store.canonicalTimeline(sessionId) ?? [];
  expect(timeline.map((e) => e.seq)).toEqual([100]);
  const text = timeline[0]?.kind === "update" ? timeline[0].update.content?.text : "";
  expect(text).toBe("hello world");

  // Delayed covered WS chunk (seq 100 < accepted watermark 101) is dropped.
  socket.receive({
    type: "event",
    envelope: {
      session_id: sessionId,
      seq: 100,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: " DUPE" } },
    },
  });
  await settle(25);
  const afterDelayed = store.canonicalTimeline(sessionId) ?? [];
  expect(afterDelayed[0]?.kind === "update" ? afterDelayed[0].update.content?.text : "").toBe(
    "hello world",
  );

  // A live context_cleared arriving while an older pre-clear snapshot is in
  // flight must never resurrect cleared rows.
  socket.receive({
    type: "event",
    envelope: {
      session_id: sessionId,
      seq: 150,
      kind: "update",
      update: { sessionUpdate: "context_cleared" },
    },
  });
  socket.receive({
    type: "snapshot",
    session_id: sessionId,
    reached_start: true,
    next_seq: 140,
    events: [
      {
        session_id: sessionId,
        seq: 100,
        kind: "update",
        update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "ghost" } },
      },
    ],
  });
  await settle(25);
  expect((store.canonicalTimeline(sessionId) ?? []).map((e) => e.seq)).toEqual([150]);

  // A standalone live chunk already folded into the incoming snapshot must not
  // survive as a duplicate row after merging onto the pre-journal base.
  const overlapId = "delta-replica-overlap-dedupe";
  await database.cache({ kind: "session", session: overlapId, state: "tail" }).save({
    receivedAt: Date.now() - 5_000,
    lastSeq: 99,
    reachedStart: true,
    events: [
      {
        session_id: overlapId,
        seq: 99,
        kind: "update",
        update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: "ask" } },
      },
    ],
  });
  store.openSession(overlapId);
  await settle(25);
  socket.receive({
    type: "event",
    envelope: {
      session_id: overlapId,
      seq: 101,
      kind: "update",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: " world" } },
    },
  });
  await settle(25);
  socket.receive({
    type: "snapshot",
    session_id: overlapId,
    reached_start: true,
    next_seq: 102,
    events: [
      {
        session_id: overlapId,
        seq: 99,
        kind: "update",
        update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: "ask" } },
      },
      {
        session_id: overlapId,
        seq: 100,
        kind: "update",
        update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "hello world" } },
      },
    ],
  });
  await settle(25);
  expect((store.canonicalTimeline(overlapId) ?? []).map((e) => e.seq)).toEqual([99, 100]);
});

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
