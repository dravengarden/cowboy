// Real IndexedDB acceptance in an empty profile/private loopback namespace.
// Workers are separate JS owners; they never open product data or send effects.
// oxlint-disable promise/avoid-new
import {
  type ClientSnapshot,
  type LocalPersistence,
  type Mutation,
  replicatedStore,
} from "@cowboy/state-sync";
import {
  createIdbPersistenceOwner,
  IdbPersistenceError,
} from "../../components/state-sync-idb/index.ts";
import { createSyncShutdown } from "./syncShutdown.ts";
import {
  createProductSyncDatabase,
  type SyncDataset,
} from "./productSyncDatabase.ts";

type Snapshot = ClientSnapshot<number>;

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
type PeerCommand =
  | { kind: "open"; dbName: string; dataset?: SyncDataset }
  | { kind: "load" }
  | { kind: "save"; snapshot: Snapshot }
  | { kind: "hold"; snapshot: Snapshot }
  | { kind: "close" };

function check(condition: boolean, label: string): asserts condition {
  if (!condition) throw new Error(label);
}

function snapshot(...ids: string[]): Snapshot {
  return {
    base: { version: 0, value: 0 },
    pending: ids.map((id) => ({ id, client: "fixture", name: "add", args: 1 })),
  };
}

function ids(value: unknown): string[] {
  check(
    value !== null && typeof value === "object" && "pending" in value,
    "snapshot envelope",
  );
  const pending = value.pending;
  check(Array.isArray(pending), "pending array");
  return pending.map((mutation) => {
    check(typeof mutation?.id === "string", "mutation identity");
    return mutation.id;
  });
}

function native<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result), {
      once: true,
    });
    request.addEventListener(
      "error",
      () => reject(new Error("native fixture request failed")),
      { once: true },
    );
  });
}

async function rejects(
  action: () => Promise<unknown>,
  code: string,
): Promise<void> {
  try {
    await action();
  } catch (error) {
    check(
      error instanceof IdbPersistenceError && error.code === code,
      `expected ${code}`,
    );
    return;
  }
  throw new Error(`missing ${code}`);
}

function interceptPut(
  effect: (tx: IDBTransaction, store: IDBObjectStore) => void,
): Pick<IDBFactory, "open"> {
  return {
    open: (name, version): IDBOpenDBRequest => {
      const request = indexedDB.open(name, version);
      request.addEventListener("success", () => {
        const db = request.result;
        const transaction = db.transaction.bind(db);
        db.transaction = (...args): IDBTransaction => {
          const tx = transaction(...args);
          const store = tx.objectStore("clients");
          const put = store.put.bind(store);
          store.put = (...values): IDBRequest<IDBValidKey> => {
            const writing = put(...values);
            writing.addEventListener("success", () => effect(tx, store), {
              once: true,
            });
            return writing;
          };
          return tx;
        };
      }, { once: true });
      return request;
    },
  };
}

/** Worker-only entry; served by the closed fixture runner, not the product. */
export function runIdbOutboxPeer(): void {
  let owner: { dispose(): Promise<void> } | undefined;
  let outbox: LocalPersistence<Snapshot> | undefined;
  let held: ReturnType<typeof deferred<void>> | undefined;
  const execute = async (command: PeerCommand): Promise<unknown> => {
    switch (command.kind) {
      case "open": {
        check(!owner, "peer opens once");
        const options = {
          dbName: command.dbName,
          factory: interceptPut((_tx, store) => {
            if (!held) return;
            // Real requests keep the write transaction alive after put success.
            // Worker termination, not a fabricated abort/result, ends this owner.
            const keepAlive = (): void => {
              store.get("queue").addEventListener("success", keepAlive, {
                once: true,
              });
            };
            keepAlive();
            held.resolve();
          }),
        };
        if (command.dataset) {
          const dataset = command.dataset;
          const scoped = createProductSyncDatabase(
            () => dataset.user_id,
            async () => dataset,
            options,
          );
          owner = scoped;
          outbox = scoped.outbox<number>({
            kind: "session",
            session: "session-a",
            state: "queue",
          });
        } else {
          const legacy = createIdbPersistenceOwner(options);
          owner = legacy;
          outbox = legacy.outbox<number>("queue");
        }
        return null;
      }
      case "load": {
        check(outbox !== undefined, "peer open");
        const value = await outbox.load();
        outbox.acceptLoadedSnapshot!(value);
        return value;
      }
      case "save":
        check(outbox !== undefined, "peer open");
        await outbox.save(command.snapshot);
        return null;
      case "hold":
        check(outbox !== undefined && !held, "peer hold once");
        held = deferred<void>();
        void outbox.save(command.snapshot).catch(() =>
          held!.reject(new Error("held write failed"))
        );
        await held.promise;
        return null;
      case "close":
        check(owner !== undefined, "peer open");
        await owner.dispose();
        return null;
    }
  };
  self.addEventListener(
    "message",
    (event: MessageEvent<{ id: number; command: PeerCommand }>) => {
      const { id, command } = event.data;
      void execute(command).then(
        (value) => self.postMessage({ id, ok: true, value }),
        () => self.postMessage({ id, ok: false }),
      );
    },
  );
}

class Peer {
  private readonly worker = new Worker("/outbox-peer.js", { type: "module" });
  private sequence = 0;
  private readonly pending = new Map<
    number,
    ReturnType<typeof deferred<unknown>>
  >();

  constructor() {
    this.worker.addEventListener("message", (event: MessageEvent) => {
      const { id, ok, value } = event.data;
      const waiter = this.pending.get(id);
      this.pending.delete(id);
      if (ok === true) waiter?.resolve(value);
      else waiter?.reject(new Error("peer fixture failed"));
    });
    this.worker.addEventListener("error", () => {
      for (const waiter of this.pending.values()) {
        waiter.reject(new Error("peer worker failed"));
      }
      this.pending.clear();
    });
  }

  call(command: PeerCommand): Promise<unknown> {
    const id = ++this.sequence;
    const waiter = deferred<unknown>();
    this.pending.set(id, waiter);
    this.worker.postMessage({ id, command });
    return waiter.promise;
  }

  terminate(): void {
    this.worker.terminate();
    for (const waiter of this.pending.values()) {
      waiter.reject(new Error("peer terminated"));
    }
    this.pending.clear();
  }
}

export async function runIdbOutboxBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  let checkpoint = "start";
  const upgradeEvents: string[] = [];
  const names: string[] = [];
  const owners: { dispose(): Promise<void> }[] = [];
  const peers: Peer[] = [];
  const finishCase = async (label: string): Promise<void> => {
    // An independent case owns its own lifetime, not a growing origin-wide
    // pool of handles. Preserve concurrent owners WITHIN every case, then drain.
    for (const peer of peers.splice(0)) peer.terminate();
    await Promise.all(owners.splice(0).map((database) => database.dispose()));
    tests.push(label);
    checkpoint = "start";
    upgradeEvents.length = 0;
  };
  const fresh = (): string => {
    const name = `cowboy-outbox-conformance-${crypto.randomUUID()}`;
    names.push(name);
    return name;
  };
  const owner = (dbName: string, factory?: Pick<IDBFactory, "open">) => {
    const result = createIdbPersistenceOwner({
      dbName,
      ...(factory ? { factory } : {}),
    });
    owners.push(result);
    return result;
  };
  const read = async (dbName: string): Promise<unknown> => {
    const observer = owner(dbName);
    try {
      return await observer.persistence("queue").load();
    } finally {
      await observer.dispose();
    }
  };
  const peer = async (dbName: string, dataset?: SyncDataset): Promise<Peer> => {
    const result = new Peer();
    peers.push(result);
    await result.call({
      kind: "open",
      dbName,
      ...(dataset ? { dataset } : {}),
    });
    return result;
  };
  const descriptor = (user = "user-a", id = "a"): SyncDataset => ({
    schema: "dravengarden.cowboy.product-sync-dataset/v1",
    dataset_id: `dataset-${id.repeat(64)}`,
    user_id: user,
    database_version: 2,
    outbox_contract: "atomic-delta-v1",
  });
  const scoped = (
    dbName: string,
    dataset = descriptor(),
    principal = () => dataset.user_id,
  ) => {
    const result = createProductSyncDatabase(principal, async () => dataset, {
      dbName,
    });
    owners.push(result);
    return result;
  };
  const scope = {
    kind: "session",
    session: "session-a",
    state: "queue",
  } as const;
  const readScoped = async (dbName: string): Promise<unknown> => {
    const observer = scoped(dbName);
    try {
      return await observer.outbox<number>(scope).load();
    } finally {
      await observer.dispose();
    }
  };
  try {
    {
      const dbName = fresh();
      const [a, b] = await Promise.all([peer(dbName), peer(dbName)]);
      await Promise.all([a.call({ kind: "load" }), b.call({ kind: "load" })]);
      const ownA: string[] = [];
      const ownB: string[] = [];
      for (let i = 0; i < 20; i++) {
        ownA.push(`a-${i}`);
        ownB.push(`b-${i}`);
        await Promise.all([
          a.call({ kind: "save", snapshot: snapshot(...ownA) }),
          b.call({ kind: "save", snapshot: snapshot(...ownB) }),
        ]);
      }
      check(
        ids(await read(dbName)).sort().join() ===
          [...ownA, ...ownB].sort().join(),
        "all independent peer additions survive",
      );
      await Promise.all([a.call({ kind: "close" }), b.call({ kind: "close" })]);
      await finishCase(
        "two independent Workers / 20 concurrent rounds / no lost additions",
      );
    }
    {
      const dbName = fresh();
      const seed = owner(dbName);
      await seed.persistence("queue", { strictWrites: true }).save(
        snapshot("a"),
      );
      await seed.dispose();
      const [a, b] = await Promise.all([peer(dbName), peer(dbName)]);
      await Promise.all([a.call({ kind: "load" }), b.call({ kind: "load" })]);
      await a.call({ kind: "save", snapshot: snapshot() });
      await b.call({ kind: "save", snapshot: snapshot("a", "b") });
      await b.call({ kind: "save", snapshot: snapshot("a", "b") });
      check(
        ids(await read(dbName)).join() === "b",
        "confirmed id never resurrected",
      );
      await Promise.all([a.call({ kind: "close" }), b.call({ kind: "close" })]);
      await finishCase(
        "cross-Worker confirmation dominates repeated stale snapshots",
      );
    }
    {
      const dbName = fresh();
      const [a, b] = [owner(dbName), owner(dbName)];
      const sent: Mutation[] = [];
      const client = (database: typeof a, clientId: string) =>
        replicatedStore({
          initial: 0,
          clientId,
          mutators: {
            add: (value: number, amount: number): number => value + amount,
          },
          send: (m) => sent.push(m),
          local: database.outbox<number>("queue"),
        });
      const first = client(a, "first");
      const second = client(b, "second");
      await Promise.all([first.hydrate(), second.hydrate()]);
      await Promise.all([
        first.mutateDurably("add", 1, "a"),
        second.mutateDurably("add", 2, "b"),
      ]);
      check(sent.length === 2, "two sends after durability");
      await Promise.all([
        createSyncShutdown(a)([first]),
        createSyncShutdown(b)([second]),
      ]);
      check(
        ids(await read(dbName)).sort().join() === "a,b",
        "disposal preserves both outboxes",
      );
      const next = owner(dbName);
      const replacement = client(next, "replacement");
      await replacement.hydrate();
      check(
        replacement.get() === 3,
        "reopened client restores both obligations",
      );
      replacement.resend();
      check(
        sent.slice(2).map((m) => m.id).sort().join() === "a,b",
        "stable retry ids",
      );
      await createSyncShutdown(next)([replacement]);
      await finishCase(
        "real replicated clients / durable sends / concurrent dispose / cold resend",
      );
    }
    {
      const dbName = fresh();
      const a = await peer(dbName);
      await a.call({ kind: "load" });
      await a.call({ kind: "hold", snapshot: snapshot("uncommitted") });
      a.terminate();
      check(
        await read(dbName) === null,
        "terminating held owner aborts real transaction",
      );
      const replacement = owner(dbName).outbox<number>("queue");
      await replacement.save(snapshot("replacement"));
      check(
        ids(await read(dbName)).join() === "replacement",
        "next owner is not blocked",
      );
      await finishCase(
        "Worker termination after put success aborts uncommitted write / reopen",
      );
    }
    {
      const dbName = fresh();
      const a = await peer(dbName);
      await a.call({ kind: "save", snapshot: snapshot("committed") });
      a.terminate();
      check(
        ids(await read(dbName)).join() === "committed",
        "committed obligation survives owner loss",
      );
      await finishCase(
        "Worker termination after commit preserves exact pending id",
      );
    }
    {
      const dbName = fresh();
      let abort = true;
      const database = owner(
        dbName,
        interceptPut((tx) => {
          if (abort) tx.abort();
        }),
      );
      const outbox = database.outbox<number>("queue");
      await rejects(() => outbox.save(snapshot("a")), "transaction_aborted");
      check(await read(dbName) === null, "no aborted bytes");
      abort = false;
      await outbox.save(snapshot("a"));
      check(
        ids(await read(dbName)).join() === "a",
        "abort did not advance baseline",
      );
      await finishCase(
        "actual put-success abort / strict failure / same-owner retry",
      );
    }
    {
      const dbName = fresh();
      const seed = owner(dbName);
      await seed.persistence("queue", { strictWrites: true }).save(
        snapshot("a"),
      );
      await seed.dispose();
      const database = owner(dbName);
      const outbox = database.outbox<number>("queue");
      const loaded = await outbox.load();
      check(ids(loaded).join() === "a", "legacy record readable");
      outbox.acceptLoadedSnapshot!(loaded);
      await outbox.save(snapshot("a", "b"));
      await database.dispose();
      const legacy = await native(indexedDB.open(dbName, 1));
      check(
        legacy.version === 1 &&
          [...legacy.objectStoreNames].join() === "clients",
        "no schema migration",
      );
      const value = await native(
        legacy.transaction("clients").objectStore("clients").get("queue"),
      );
      check(
        Object.keys(value).sort().join() === "base,pending" &&
          ids(value).join() === "a,b",
        "exact v1 envelope retained",
      );
      legacy.close();
      await finishCase(
        "legacy v1 record adoption / exact envelope and schema remain readable",
      );
    }
    {
      const dbName = fresh();
      const database = owner(dbName);
      const outbox = database.outbox<number>("queue");
      await outbox.save(snapshot("a"));
      const upgrade = await native(indexedDB.open(dbName, 2));
      check(database.lifecycle.connections === 0, "old connection retired");
      await rejects(() => outbox.save(snapshot("a", "b")), "open_failed");
      const value = await native(
        upgrade.transaction("clients").objectStore("clients").get("queue"),
      );
      check(ids(value).join() === "a", "incompatible version never rewritten");
      upgrade.close();
      await finishCase(
        "versionchange retires writer / future schema never downgraded",
      );
    }
    {
      const dbName = fresh();
      const seed = owner(dbName);
      await seed.persistence("queue", { strictWrites: true }).save({
        invalid: true,
      });
      await seed.dispose();
      const outbox = owner(dbName).outbox<number>("queue");
      await rejects(() => outbox.load(), "snapshot_invalid");
      await rejects(() => outbox.save(snapshot()), "snapshot_invalid");
      check(
        JSON.stringify(await read(dbName)) === '{"invalid":true}',
        "corrupt evidence retained",
      );
      await finishCase(
        "corrupt read is not an empty outbox / later writes stay fenced",
      );
    }
    {
      const dbName = fresh();
      const a = owner(dbName);
      const b = owner(dbName);
      const keys = [
        "workspace-a:g1",
        "workspace-a:g2",
        "workspace-b:g1",
        "workspace-b:g2",
      ];
      await Promise.all(
        keys.map((key, index) =>
          (index % 2 ? a : b).outbox<number>(key).save(snapshot(key))
        ),
      );
      const observer = owner(dbName);
      for (const key of keys) {
        check(
          ids(await observer.persistence(key).load()).join() === key,
          "record isolation",
        );
      }
      await finishCase(
        "independent record keys coexist / no workspace-generation aliasing in storage",
      );
    }
    {
      const dbName = fresh();
      checkpoint = "v1 seed";
      const observed: Pick<IDBFactory, "open"> = {
        open: (name, version) => {
          const opened = performance.now();
          const request = indexedDB.open(name, version);
          for (
            const event of ["upgradeneeded", "blocked", "error", "success"]
          ) {
            request.addEventListener(event, () => {
              upgradeEvents.push(
                `${version}/${event}/${
                  Math.round(performance.now() - opened)
                }ms`,
              );
            });
          }
          return request;
        },
      };
      const old = owner(dbName, observed);
      const legacyKey = "cowboy:sync:queue:session-a";
      const legacy = old.persistence<Snapshot>(legacyKey, {
        strictWrites: true,
      });
      await legacy.save(snapshot("unowned"));
      checkpoint = "v2 upgrade/load";
      const current = createProductSyncDatabase(
        () => "user-a",
        async () => descriptor(),
        { dbName, factory: observed },
      );
      owners.push(current);
      const outbox = current.outbox<number>(scope);
      const loaded = await outbox.load();
      check(loaded === null, "unowned v1 data not adopted");
      outbox.acceptLoadedSnapshot!(loaded);
      checkpoint = "v2 write";
      await outbox.save(snapshot("owned"));
      check(old.lifecycle.connections === 0, "v1 connection closed by upgrade");
      await rejects(() => legacy.save(snapshot("blind")), "open_failed");
      checkpoint = "legacy export";
      const exported = JSON.parse(await current.exportLegacy(legacyKey));
      check(
        current.lifecycle.connections === 0 &&
          current.lifecycle.transactions === 0,
        "idle product connections release only after terminal transactions",
      );
      check(
        exported.replay_authorized === false &&
          ids(exported.value).join() === "unowned",
        "old data preserved as unowned evidence",
      );
      checkpoint = "v2 independent reader";
      const independent = createProductSyncDatabase(
        () => "user-a",
        async () => descriptor(),
        { dbName, factory: observed },
      );
      owners.push(independent);
      try {
        check(
          ids(await independent.outbox<number>(scope).load()).join() ===
            "owned",
          "owned data independent",
        );
      } catch (error) {
        throw new Error(
          `${String(error)}; previous=${
            JSON.stringify(current.lifecycle)
          }; next=${JSON.stringify(independent.lifecycle)}`,
        );
      }
      await independent.dispose();
      await finishCase(
        "product v2 upgrade / v1 writer fenced / legacy bytes retained without adoption",
      );
    }
    {
      const dbName = fresh();
      const [a, b] = await Promise.all([
        peer(dbName, descriptor()),
        peer(dbName, descriptor()),
      ]);
      await Promise.all([a.call({ kind: "load" }), b.call({ kind: "load" })]);
      const aa: string[] = [], bb: string[] = [];
      for (let i = 0; i < 20; i++) {
        aa.push(`owned-a-${i}`);
        bb.push(`owned-b-${i}`);
        await Promise.all([
          a.call({ kind: "save", snapshot: snapshot(...aa) }),
          b.call({ kind: "save", snapshot: snapshot(...bb) }),
        ]);
      }
      check(
        ids(await readScoped(dbName)).sort().join() ===
          [...aa, ...bb].sort().join(),
        "owned peers preserve all forty identities",
      );
      await Promise.all([a.call({ kind: "close" }), b.call({ kind: "close" })]);
      await finishCase(
        "owned v2 dataset / independent Worker transactions / forty concurrent additions",
      );
    }
    {
      const dbName = fresh();
      const a = scoped(dbName),
        b = scoped(dbName, descriptor("user-b", "b")),
        service = scoped(dbName, descriptor("user-a", "c"));
      const outbox = a.outbox<number>(scope);
      outbox.acceptLoadedSnapshot!(await outbox.load());
      await outbox.save(snapshot("only-a"));
      check(
        await b.outbox(scope).load() === null &&
          await service.outbox(scope).load() === null,
        "other Service and user isolated",
      );
      check(
        await a.outbox({ ...scope, session: "session-b" }).load() === null,
        "other session isolated",
      );
      check(
        await a.outbox({ ...scope, state: "mobile-review" }).load() === null,
        "other contract isolated",
      );
      check(
        (await a.queueSessions()).join() === "session-a" &&
          (await b.queueSessions()).length === 0,
        "enumeration stays in exact dataset",
      );
      await a.dispose();
      check(
        ids(await readScoped(dbName)).join() === "only-a",
        "new owner uses existing logical dataset, not empty generation namespace",
      );
      await finishCase(
        "Service / principal / Session / contract isolation with stable data on owner replacement",
      );
    }
    {
      const dbName = fresh();
      const a = await peer(dbName, descriptor());
      await a.call({ kind: "load" });
      await a.call({ kind: "hold", snapshot: snapshot("uncommitted-owned") });
      a.terminate();
      check(
        await readScoped(dbName) === null,
        "interrupted owned write aborted",
      );
      const b = await peer(dbName, descriptor());
      await b.call({ kind: "load" });
      await b.call({ kind: "save", snapshot: snapshot("committed-owned") });
      b.terminate();
      check(
        ids(await readScoped(dbName)).join() === "committed-owned",
        "committed owned data survives Worker loss",
      );
      await finishCase(
        "owned v2 Worker termination before/after commit / exact cold-reopen obligation",
      );
    }
    {
      const dbName = fresh();
      const current = scoped(dbName);
      const outbox = current.outbox<number>(scope);
      outbox.acceptLoadedSnapshot!(await outbox.load());
      await outbox.save(snapshot("retained-v2"));
      const future = await native(indexedDB.open(dbName, 3));
      try {
        await rejects(
          () => outbox.save(snapshot("future-write")),
          "open_failed",
        );
        check(
          current.lifecycle.connections === 0,
          "v2 writer retired at next reader floor",
        );
        const key =
          `cowboy:dataset:${descriptor().dataset_id}:session:session-a:queue`;
        const value = await native(
          future.transaction("clients").objectStore("clients").get(key),
        );
        check(
          ids(value).join() === "retained-v2",
          "future reader sees unmodified evidence",
        );
      } finally {
        future.close();
      }
      await finishCase(
        "future v3 upgrade fences actual v2 writer without rewriting or deleting its record",
      );
    }
    {
      const dbName = fresh();
      let principal = "user-a";
      const current = scoped(dbName, descriptor(), () => principal);
      const outbox = current.outbox<number>(scope);
      outbox.acceptLoadedSnapshot!(await outbox.load());
      await outbox.save(snapshot("before-change"));
      principal = "user-b";
      let refused = 0;
      try {
        await outbox.save(snapshot("wrong-user"));
      } catch {
        refused++;
      }
      principal = "user-a";
      try {
        await outbox.save(snapshot("revived"));
      } catch {
        refused++;
      }
      check(
        refused === 2 &&
          ids(await readScoped(dbName)).join() === "before-change",
        "observed principal change is terminal",
      );
      await finishCase(
        "actual database / principal change and ABA / old writer cannot revive",
      );
    }
  } catch (error) {
    throw new Error(
      `outbox case ${tests.length + 1} (${checkpoint}; ${
        upgradeEvents.join(",")
      }): ${String(error)}`,
    );
  } finally {
    for (const peer of peers) peer.terminate();
    await Promise.all(owners.map((database) => database.dispose()));
    // Only these exclusively generated disposable fixture databases.
    await Promise.all(
      names.map((name) => native(indexedDB.deleteDatabase(name))),
    );
  }
  return tests;
}
