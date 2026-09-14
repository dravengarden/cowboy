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
  | { kind: "open"; dbName: string }
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
  let owner: ReturnType<typeof createIdbPersistenceOwner> | undefined;
  let outbox: LocalPersistence<Snapshot> | undefined;
  let held: ReturnType<typeof deferred<void>> | undefined;
  const execute = async (command: PeerCommand): Promise<unknown> => {
    switch (command.kind) {
      case "open":
        check(!owner, "peer opens once");
        owner = createIdbPersistenceOwner({
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
        });
        outbox = owner.outbox<number>("queue");
        return null;
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
  const names: string[] = [];
  const owners: ReturnType<typeof createIdbPersistenceOwner>[] = [];
  const peers: Peer[] = [];
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
  const peer = async (dbName: string): Promise<Peer> => {
    const result = new Peer();
    peers.push(result);
    await result.call({ kind: "open", dbName });
    return result;
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
      tests.push(
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
      tests.push(
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
      tests.push(
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
      tests.push(
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
      tests.push("Worker termination after commit preserves exact pending id");
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
      tests.push(
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
      tests.push(
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
      tests.push(
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
      tests.push(
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
      tests.push(
        "independent record keys coexist / no workspace-generation aliasing in storage",
      );
    }
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
