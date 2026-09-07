import { assertEquals } from "jsr:@std/assert";
import { idbPersistence } from "../../components/state-sync-idb/idb.ts";

class FakeRequest<R> extends EventTarget {
  result!: R;
  error: DOMException | null = null;
}

class FakeTransaction extends EventTarget {
  error: DOMException | null = null;

  objectStore(): IDBObjectStore {
    return {
      put: (_value: unknown, key: IDBValidKey): IDBRequest<IDBValidKey> => {
        const request = new FakeRequest<IDBValidKey>();
        queueMicrotask(() => {
          request.result = key;
          request.dispatchEvent(new Event("success"));
          queueMicrotask(() => this.dispatchEvent(new Event("complete")));
        });
        return request as IDBRequest<IDBValidKey>;
      },
    } as IDBObjectStore;
  }
}

class FakeDatabase extends EventTarget {
  closing = false;
  closeCalls = 0;
  readonly objectStoreNames = {
    contains: (): boolean => true,
  } as DOMStringList;

  createObjectStore(): IDBObjectStore {
    return {} as IDBObjectStore;
  }

  transaction(): IDBTransaction {
    if (this.closing) {
      throw new DOMException(
        "The database connection is closing.",
        "InvalidStateError",
      );
    }
    return new FakeTransaction() as IDBTransaction;
  }

  close(): void {
    this.closing = true;
    this.closeCalls += 1;
  }
}

class FakeIndexedDb {
  readonly databases: FakeDatabase[] = [];

  open(): IDBOpenDBRequest {
    const request = new FakeRequest<IDBDatabase>();
    const database = new FakeDatabase();
    this.databases.push(database);
    queueMicrotask(() => {
      request.result = database as IDBDatabase;
      request.dispatchEvent(new Event("upgradeneeded"));
      request.dispatchEvent(new Event("success"));
    });
    return request as IDBOpenDBRequest;
  }
}

Deno.test("strict IndexedDB writes recover from a cached closing connection", async () => {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "indexedDB");
  const factory = new FakeIndexedDb();
  Object.defineProperty(globalThis, "indexedDB", {
    configurable: true,
    value: factory,
  });

  try {
    const persistence = idbPersistence<{ value: number }>("queue", {
      dbName: "closing-connection-test",
      strictWrites: true,
    });

    await persistence.save({ value: 1 });
    assertEquals(factory.databases.length, 1);

    // Safari/Chromium can mark a connection close-pending before delivering
    // its `close` event. Reusing that cached handle throws InvalidStateError.
    factory.databases[0]!.closing = true;
    await persistence.save({ value: 2 });
    assertEquals(factory.databases.length, 2);

    // A delayed close event from the retired handle must not evict its live
    // replacement.
    factory.databases[0]!.dispatchEvent(new Event("close"));
    await persistence.save({ value: 3 });
    assertEquals(factory.databases.length, 2);

    // A version upgrade asks this page to release its connection. The next
    // durability barrier should reopen instead of retaining a closed handle.
    factory.databases[1]!.dispatchEvent(new Event("versionchange"));
    assertEquals(factory.databases[1]!.closeCalls, 1);
    await persistence.save({ value: 4 });
    assertEquals(factory.databases.length, 3);
  } finally {
    if (descriptor === undefined) {
      delete (globalThis as { indexedDB?: IDBFactory }).indexedDB;
    } else {
      Object.defineProperty(globalThis, "indexedDB", descriptor);
    }
  }
});
