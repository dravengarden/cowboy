import { assertEquals } from "jsr:@std/assert";
import {
  type ClientSnapshot,
  createArbiter,
  type LocalPersistence,
  type Mutation,
  replicatedStore,
  snapshotPatch,
} from "@cowboy/state-sync";
import { durableDeliveryAttempt } from "./durableDelivery.ts";
import {
  emptyQueueValue,
  type QueueValue,
  queueMutators as deliveryMutators,
} from "./queueMutators.ts";

interface QueueState {
  readonly rows: readonly string[];
}

const queueMutators = {
  submit: (state: QueueState, args: { text: string }): QueueState => ({
    rows: [...state.rows, args.text],
  }),
};

function memoryPersistence<S>(): LocalPersistence<S> & { snapshot: () => S | null } {
  let value: S | null = null;
  return {
    load: (): Promise<S | null> => Promise.resolve(value === null ? null : structuredClone(value)),
    save: (next): Promise<void> => {
      value = structuredClone(next);
      return Promise.resolve();
    },
    snapshot: (): S | null => value === null ? null : structuredClone(value),
  };
}

Deno.test("offline durable send survives reload and resends the same cmid after reconnect", async () => {
  const persistence = memoryPersistence<ClientSnapshot<QueueState>>();
  const sent: Mutation[] = [];
  let connected = false;
  const transport = (mutation: Mutation): void => {
    const attempt = durableDeliveryAttempt(connected);
    if (attempt.armConfirmationTimeout) sent.push(mutation);
  };

  const beforeReload = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "browser-tab",
    mutators: queueMutators,
    initial: { rows: [] },
    local: persistence,
    send: transport,
  });
  await beforeReload.mutateDurably(
    "submit",
    { text: "survive reconnect" },
    "cmid-1",
  );

  assertEquals(sent, []);
  assertEquals(persistence.snapshot()?.pending.map((mutation) => mutation.id), ["cmid-1"]);

  // Model a browser reload: construct a new store over the same persisted
  // IndexedDB-shaped record, hydrate before opening the replacement socket,
  // then resend every still-unconfirmed mutation when WebSocket reaches OPEN.
  const afterReload = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "browser-tab-reloaded",
    mutators: queueMutators,
    initial: { rows: [] },
    local: persistence,
    send: transport,
  });
  await afterReload.hydrate();
  assertEquals(afterReload.get().rows, ["survive reconnect"]);
  assertEquals(afterReload.pending().map((mutation) => mutation.id), ["cmid-1"]);

  connected = true;
  afterReload.resend();
  assertEquals(sent.map((mutation) => mutation.id), ["cmid-1"]);

  const arbiter = createArbiter({ mutators: queueMutators, initial: { rows: [] } });
  const patch = arbiter.receive(sent[0]!);
  if (patch === null) throw new Error("first delivery was unexpectedly deduplicated");
  afterReload.applyPatch(patch);
  await afterReload.flush();

  assertEquals(afterReload.pending(), []);
  assertEquals(afterReload.get().rows, ["survive reconnect"]);
  assertEquals(arbiter.receive(sent[0]!), null);

  // A second reload proves the acknowledgement removed the mutation from the
  // durable outbox; it cannot be replayed into a duplicate prompt later.
  const afterAckReload = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "browser-tab-after-ack",
    mutators: queueMutators,
    initial: { rows: [] },
    local: persistence,
    send: transport,
  });
  await afterAckReload.hydrate();
  afterAckReload.resend();
  assertEquals(afterAckReload.get().rows, ["survive reconnect"]);
  assertEquals(afterAckReload.pending(), []);
  assertEquals(sent.map((mutation) => mutation.id), ["cmid-1"]);
});

Deno.test("a transcript prompt remains visible across reload until its user echo", async () => {
  type Row = { id: string; text: string; cmid: string };
  const persistence = memoryPersistence<ClientSnapshot<QueueValue<Row>>>();
  const row: Row = { id: "opt-cmid-2", text: "still visible", cmid: "cmid-2" };

  const beforeReload = replicatedStore<QueueValue<Row>, typeof deliveryMutators>({
    clientId: "browser-tab",
    mutators: deliveryMutators,
    initial: emptyQueueValue<Row>(),
    local: persistence,
    send: () => {},
  });
  await beforeReload.mutateDurably("submitPrompt", { row }, row.cmid);

  const afterReload = replicatedStore<QueueValue<Row>, typeof deliveryMutators>({
    clientId: "browser-tab-reloaded",
    mutators: deliveryMutators,
    initial: emptyQueueValue<Row>(),
    local: persistence,
    send: () => {},
  });
  await afterReload.hydrate();
  assertEquals(afterReload.get().inFlight, [row]);
  assertEquals(afterReload.pending().map((mutation) => mutation.id), [row.cmid]);

  // The bootstrap/live user echo acknowledges the same cmid. Retiring the
  // local mutation removes the bubble and its persisted replay obligation.
  afterReload.confirm([row.cmid]);
  await afterReload.flush();
  assertEquals(afterReload.get().inFlight, []);
  assertEquals(afterReload.pending(), []);

  const afterEchoReload = replicatedStore<QueueValue<Row>, typeof deliveryMutators>({
    clientId: "browser-tab-after-echo",
    mutators: deliveryMutators,
    initial: emptyQueueValue<Row>(),
    local: persistence,
    send: () => {},
  });
  await afterEchoReload.hydrate();
  assertEquals(afterEchoReload.get().inFlight, []);
  assertEquals(afterEchoReload.pending(), []);
});

Deno.test("durable mutation commits its outbox before transport delivery", async () => {
  let releaseSave: (() => void) | undefined;
  let noteSaveStarted: (() => void) | undefined;
  const saveStarted = new Promise<void>((resolve) => {
    noteSaveStarted = resolve;
  });
  const persistence: LocalPersistence<ClientSnapshot<QueueState>> = {
    load: () => Promise.resolve(null),
    save: () => {
      noteSaveStarted?.();
      return new Promise<void>((resolve) => {
        releaseSave = resolve;
      });
    },
  };
  const sent: string[] = [];
  let paints = 0;
  const store = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "barrier",
    mutators: queueMutators,
    initial: { rows: [] },
    local: persistence,
    send: (mutation) => sent.push(mutation.id),
    onChange: () => {
      paints += 1;
    },
  });

  const commit = store.mutateDurably("submit", { text: "keep me" }, "cmid-barrier");
  await saveStarted;
  assertEquals(sent, [], "transport must not precede the IndexedDB commit");
  assertEquals(store.get().rows, ["keep me"]);
  assertEquals(paints, 1, "subscribers must paint before the durable write resolves");
  releaseSave?.();
  await commit;
  assertEquals(sent, ["cmid-barrier"]);
});

Deno.test("late hydration preserves live authority and restores only unconfirmed outbox rows", async () => {
  let finishLoad: ((snapshot: ClientSnapshot<QueueState>) => void) | undefined;
  let persisted: ClientSnapshot<QueueState> | null = null;
  const persistence: LocalPersistence<ClientSnapshot<QueueState>> = {
    load: () => new Promise((resolve) => {
      finishLoad = resolve;
    }),
    save: (snapshot) => {
      persisted = structuredClone(snapshot);
      return Promise.resolve();
    },
  };
  const sent: string[] = [];
  const store = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "slow-indexeddb",
    mutators: queueMutators,
    initial: { rows: [] },
    local: persistence,
    send: (mutation) => sent.push(mutation.id),
  });

  store.applyPatch(
    snapshotPatch(7, { rows: ["live server row"] }, ["cmid-already-confirmed"]),
  );
  // Cowboy can receive this patch while IndexedDB is still enumerating keys;
  // the per-session hydrate call starts only after the live base already exists.
  const hydration = store.hydrate();
  store.mutate("submit", { text: "typed after socket open" }, "cmid-live");

  finishLoad?.({
    base: { version: 2, value: { rows: ["stale cached row"] } },
    pending: [
      {
        id: "cmid-offline",
        client: "previous-page",
        name: "submit",
        args: { text: "restored offline row" },
      },
      {
        id: "cmid-already-confirmed",
        client: "previous-page",
        name: "submit",
        args: { text: "must not resurrect" },
      },
    ],
  });
  await hydration;

  assertEquals(store.version(), 7);
  assertEquals(store.get().rows, [
    "live server row",
    "restored offline row",
    "typed after socket open",
  ]);
  assertEquals(store.pending().map((mutation) => mutation.id), [
    "cmid-offline",
    "cmid-live",
  ]);
  assertEquals(persisted?.base.version, 7);
  assertEquals(persisted?.pending.map((mutation) => mutation.id), [
    "cmid-offline",
    "cmid-live",
  ]);
  assertEquals(sent, ["cmid-live"]);
});

Deno.test("failed durable mutation keeps the source editor authoritative", async () => {
  const sent: string[] = [];
  const store = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "failed-barrier",
    mutators: queueMutators,
    initial: { rows: [] },
    local: {
      load: () => Promise.resolve(null),
      save: () => Promise.reject(new Error("quota unavailable")),
    },
    send: (mutation) => sent.push(mutation.id),
  });

  let failed = false;
  try {
    await store.mutateDurably("submit", { text: "do not clear" }, "cmid-failed");
  } catch {
    failed = true;
  }
  assertEquals(failed, true);
  assertEquals(sent, []);
  assertEquals(store.pending(), []);
  assertEquals(store.get().rows, []);
});

Deno.test("failed durable discard restores the pending delivery", async () => {
  let snapshot: ClientSnapshot<QueueState> | null = null;
  let rejectWrites = false;
  const persistence: LocalPersistence<ClientSnapshot<QueueState>> = {
    load: () => Promise.resolve(snapshot === null ? null : structuredClone(snapshot)),
    save: (next) => {
      if (rejectWrites) return Promise.reject(new Error("disk unavailable"));
      snapshot = structuredClone(next);
      return Promise.resolve();
    },
  };
  const store = replicatedStore<QueueState, typeof queueMutators>({
    clientId: "discard-barrier",
    mutators: queueMutators,
    initial: { rows: [] },
    local: persistence,
    send: () => {},
  });
  await store.mutateDurably("submit", { text: "retain on failure" }, "cmid-discard");

  rejectWrites = true;
  let failed = false;
  const discard = store.confirmDurably(["cmid-discard"]);
  store.mutate("submit", { text: "concurrent mutation" }, "cmid-concurrent");
  try {
    await discard;
  } catch {
    failed = true;
  }
  assertEquals(failed, true);
  assertEquals(store.pending().map((mutation) => mutation.id), [
    "cmid-discard",
    "cmid-concurrent",
  ]);
  assertEquals(store.get().rows, ["retain on failure", "concurrent mutation"]);
  assertEquals(snapshot?.pending.map((mutation) => mutation.id), ["cmid-discard"]);

  rejectWrites = false;
  await store.confirmDurably(["cmid-discard", "cmid-concurrent"]);
  assertEquals(store.pending(), []);
  assertEquals(store.get().rows, []);
  assertEquals(snapshot?.pending, []);
});
