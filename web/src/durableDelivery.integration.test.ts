import { assertEquals } from "jsr:@std/assert";
import {
  type ClientSnapshot,
  createArbiter,
  type LocalPersistence,
  type Mutation,
  replicatedStore,
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
  beforeReload.mutate("submit", { text: "survive reconnect" }, "cmid-1");
  await beforeReload.flush();

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
  beforeReload.mutate("submitPrompt", { row }, row.cmid);
  await beforeReload.flush();

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
