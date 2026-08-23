import { assertEquals } from "jsr:@std/assert";
import {
  type ClientSnapshot,
  createArbiter,
  type LocalPersistence,
  type Mutation,
  replicatedStore,
} from "@cowboy/state-sync";
import { durableDeliveryAttempt } from "./durableDelivery.ts";

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
