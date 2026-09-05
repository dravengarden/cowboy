import { assertEquals } from "jsr:@std/assert";
import {
  type ClientSnapshot,
  createArbiter,
  type LocalPersistence,
  type Mutation,
  replicatedStore,
  snapshotPatch,
} from "@cowboy/state-sync";
import { discardDurableDelivery, durableDeliveryAttempt, sendAfterDurableSnapshot } from "./durableDelivery.ts";
import {
  type DraftActivation,
  draftActivationSourceId,
  emptyQueueValue,
  type QueueValue,
  queueMutators as deliveryMutators,
  settledTransitionIds,
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

for (const destination of ["queue", "transcript"] as const) {
  Deno.test(`unconfirmed draft moves to ${destination} once across reload and late creation ack`, async () => {
    const row = {
      id: "opt-create-draft",
      cmid: "create-draft",
      text: "draft with an image",
      attachments: [{ block: { type: "image", data: "cGl4ZWw=", mimeType: "image/png" } }],
    };
    type Row = typeof row;
    const activation: DraftActivation<Row> = {
      id: row.id,
      sourceCmid: row.cmid,
      row: { ...row, id: "opt-send-draft", cmid: "send-draft" },
      destination,
    };
    const persistence = memoryPersistence<ClientSnapshot<QueueValue<Row>>>();
    const sent: string[] = [];
    const makeStore = () => {
      const store = replicatedStore<QueueValue<Row>, typeof deliveryMutators>({
        clientId: "draft-send-reload",
        mutators: deliveryMutators,
        initial: emptyQueueValue<Row>(),
        local: persistence,
        send: (mutation) => {
          if (mutation.name !== "activateDraft") return;
          const id = draftActivationSourceId(mutation.args as DraftActivation, store.baseValue().drafts);
          if (id !== null) sent.push(id);
        },
      });
      return store;
    };
    const beforeReload = makeStore();
    await beforeReload.mutateDurably("addDraft", { row }, row.cmid);
    await beforeReload.mutateDurably("activateDraft", activation, "send-draft");
    assertEquals(beforeReload.get().drafts, []);
    assertEquals(sent, [], "no submit copy and no activation using a local id");
    assertEquals(persistence.snapshot()?.pending.map((mutation) => mutation.name), [
      "addDraft", "activateDraft",
    ]);

    const reloaded = makeStore();
    await reloaded.hydrate();
    const target = destination === "queue" ? reloaded.get().queue : reloaded.get().inFlight;
    assertEquals(target, [activation.row]);
    assertEquals(reloaded.get().drafts, []);
    // An unrelated empty snapshot before creation is not a send acknowledgement.
    assertEquals(settledTransitionIds(reloaded.pending(), emptyQueueValue<Row>()), []);
    reloaded.resend();
    assertEquals(sent, []);

    const canonical = { ...row, id: "server-draft-id" };
    const created = { ...emptyQueueValue<Row>(), drafts: [canonical] };
    reloaded.applyPatch(snapshotPatch(1, created, [row.cmid]));
    assertEquals(reloaded.get().drafts, [], "the late echo cannot resurrect the sent draft");
    assertEquals(settledTransitionIds(reloaded.pending(), created), []);
    // Reload after the creation ack, with only activation still pending. The
    // source id must come from the hydrated base even when this session is not
    // opened, so no fresh focused-session bootstrap is available to supply it.
    await reloaded.flush();
    const unfocused = makeStore();
    await unfocused.hydrate();
    assertEquals(unfocused.get().drafts, []);
    assertEquals(unfocused.baseValue().drafts, [canonical]);
    unfocused.applyPatch(snapshotPatch(0, emptyQueueValue<Row>(), []));
    assertEquals(unfocused.baseValue().drafts, [canonical], "stale snapshots cannot erase the source id");
    unfocused.resend();
    assertEquals(sent, [canonical.id], "send moves the same authoritative draft");

    const consumed = emptyQueueValue<Row>();
    unfocused.confirm(settledTransitionIds(unfocused.pending(), consumed));
    unfocused.applyPatch(snapshotPatch(2, consumed, []));
    await unfocused.flush();
    assertEquals(unfocused.pending(), []);
    const afterSend = makeStore();
    await afterSend.hydrate();
    afterSend.resend();
    assertEquals(afterSend.get(), consumed);
    assertEquals(sent, [canonical.id]);
  });
}

Deno.test("failed durable draft activation retains its original text and attachments", async () => {
  const row = { id: "draft-source", cmid: "draft-create", text: "keep original", attachments: ["image-bytes"] };
  let rejectWrites = false;
  const sent: string[] = [];
  const store = replicatedStore<QueueValue<typeof row>, typeof deliveryMutators>({
    clientId: "draft-send-quota",
    mutators: deliveryMutators,
    initial: { ...emptyQueueValue<typeof row>(), drafts: [row] },
    local: {
      load: () => Promise.resolve(null),
      save: () => rejectWrites ? Promise.reject(new Error("quota exceeded")) : Promise.resolve(),
    },
    send: (mutation) => sent.push(mutation.name),
  });
  rejectWrites = true;
  const error = await store.mutateDurably("activateDraft", {
    id: row.id, row: { ...row, id: "opt-send", cmid: "send-op" }, destination: "transcript",
  }, "send-op").then(() => null, (reason: Error) => reason.message);
  assertEquals(error, "quota exceeded");
  assertEquals(store.get().drafts, [row]);
  assertEquals(store.get().inFlight, []);
  assertEquals(sent, []);
  rejectWrites = false;
  await store.flush();
});

Deno.test("a crash immediately after activation cannot replay its consumed draft creation", async () => {
  const row = { id: "opt-created", cmid: "created", text: "send exactly once" };
  type Row = typeof row;
  const persistence = memoryPersistence<ClientSnapshot<QueueValue<Row>>>();
  const sent: string[] = [];
  const makeStore = () => replicatedStore<QueueValue<Row>, typeof deliveryMutators>({
    clientId: "crash-at-activation",
    mutators: deliveryMutators,
    initial: emptyQueueValue<Row>(),
    local: persistence,
    send: (mutation) => sent.push(mutation.name),
  });
  const store = makeStore();
  await store.mutateDurably("addDraft", { row }, row.cmid);
  await store.mutateDurably("activateDraft", {
    id: row.id, sourceCmid: row.cmid, row: { ...row, id: "opt-activate", cmid: "activate" },
  }, "activate");
  store.applyPatch(snapshotPatch(1, {
    ...emptyQueueValue<Row>(), drafts: [{ ...row, id: "server-draft" }],
  }, [row.cmid]));
  assertEquals(persistence.snapshot()?.pending.map((mutation) => mutation.name), ["addDraft", "activateDraft"]);
  await sendAfterDurableSnapshot(store, "activate", () => {
    assertEquals(persistence.snapshot()?.pending.map((mutation) => mutation.name), ["activateDraft"]);
  });

  // No debounce timer, pagehide, or final activation acknowledgement gets a
  // chance to run before this reload. The server has already consumed the draft.
  const reloaded = makeStore();
  await reloaded.hydrate();
  const consumed = emptyQueueValue<Row>();
  const settled = settledTransitionIds(reloaded.pending(), consumed);
  assertEquals(settled, ["activate"]);
  reloaded.confirm(settled);
  reloaded.applyPatch(snapshotPatch(2, consumed, []), { force: true });
  sent.length = 0;
  reloaded.resend();
  assertEquals(sent, []);
  assertEquals(reloaded.get(), consumed);
  await reloaded.flush();
});

Deno.test("discard rollback after a skipped activation restores an explicitly retryable row", async () => {
  const firstWrite = Promise.withResolvers<void>();
  const writeStarted = Promise.withResolvers<void>();
  let racing = false;
  let writes = 0;
  let sent = 0;
  let status = "committing";
  const row = { id: "canonical-draft", text: "keep after failed discard" };
  const store = replicatedStore<QueueValue<typeof row>, typeof deliveryMutators>({
    clientId: "discard-rollback",
    mutators: deliveryMutators,
    initial: { ...emptyQueueValue<typeof row>(), drafts: [row] },
    local: {
      load: () => Promise.resolve(null),
      save: () => {
        if (!racing) return Promise.resolve();
        writes++;
        if (writes === 1) {
          writeStarted.resolve();
          return firstWrite.promise;
        }
        return writes === 2 ? Promise.reject(new Error("discard write failed")) : Promise.resolve();
      },
    },
    send: () => {},
  });
  await store.mutateDurably("activateDraft", {
    id: row.id, row: { ...row, id: "opt-activate" }, destination: "transcript",
  }, "activate");
  racing = true;
  const sending = sendAfterDurableSnapshot(store, "activate", () => sent++);
  await writeStarted.promise;
  const discarding = discardDurableDelivery(store, "activate", () => { status = "failed"; })
    .then(() => "ok", (error: Error) => error.message);
  assertEquals(store.pending(), []);
  firstWrite.resolve();
  await sending;
  assertEquals(await discarding, "discard write failed");
  assertEquals(sent, 0, "do not silently send after the user tried to cancel");
  assertEquals(status, "failed", "rollback must not leave an undispatchable committing row");
  assertEquals(store.pending().map((mutation) => mutation.id), ["activate"]);
  assertEquals(store.get().inFlight[0]?.text, row.text);
  // The restored row offers Retry. A fresh durable barrier can now send it.
  await sendAfterDurableSnapshot(store, "activate", () => sent++);
  assertEquals(sent, 1);
});
