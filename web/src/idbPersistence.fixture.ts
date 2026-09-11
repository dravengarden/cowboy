// Deterministic event ordering for ownership tests; browser acceptance is a
// separate gate. This fake does not claim to implement IndexedDB's algorithm.
class TrackedTarget extends EventTarget {
  readonly listeners = new Map<
    string,
    Set<EventListenerOrEventListenerObject>
  >();

  override addEventListener(
    type: string,
    callback: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void {
    super.addEventListener(type, callback, options);
    if (!callback) return;
    const entries = this.listeners.get(type) ?? new Set();
    entries.add(callback);
    this.listeners.set(type, entries);
  }

  override removeEventListener(
    type: string,
    callback: EventListenerOrEventListenerObject | null,
    options?: boolean | EventListenerOptions,
  ): void {
    super.removeEventListener(type, callback, options);
    if (callback) this.listeners.get(type)?.delete(callback);
  }

  get listenerCount(): number {
    return [...this.listeners.values()].reduce(
      (sum, entries) => sum + entries.size,
      0,
    );
  }
}

export class FakeRequest<R> extends TrackedTarget {
  result!: R;
  error: DOMException | null = null;
  transaction: IDBTransaction | null = null;

  succeed(value: R): void {
    this.result = value;
    this.dispatchEvent(new Event("success"));
  }
}

export class FakeTransaction extends TrackedTarget {
  error: DOMException | null = null;
  readonly requests: FakeRequest<unknown>[] = [];
  abortCalls = 0;
  finished = false;
  private readonly writes = new Map<IDBValidKey, unknown>();

  constructor(readonly database: FakeDatabase) {
    super();
  }

  objectStore(): IDBObjectStore {
    const make = (value: unknown): IDBRequest<unknown> => {
      if (this.database.throwRequest) {
        throw new DOMException("private payload", "DataCloneError");
      }
      const request = new FakeRequest<unknown>();
      request.result = value;
      this.requests.push(request);
      if (this.database.factory.autoTransactions) {
        queueMicrotask(() => {
          request.succeed(value);
          queueMicrotask(() => this.complete());
        });
      }
      return request as unknown as IDBRequest<unknown>;
    };
    return {
      put: (value: unknown, key: IDBValidKey): IDBRequest<unknown> => {
        const request = make(key);
        this.writes.set(key, value);
        return request;
      },
      get: (key: IDBValidKey): IDBRequest<unknown> =>
        make(this.database.factory.data.get(key)),
      getAllKeys: (): IDBRequest<unknown> =>
        make([...this.database.factory.data.keys()]),
    } as IDBObjectStore;
  }

  complete(): void {
    if (this.finished) return;
    this.finished = true;
    for (const [key, value] of this.writes) {
      this.database.factory.data.set(key, value);
    }
    this.dispatchEvent(new Event("complete"));
  }

  abort(): void {
    this.abortCalls++;
    queueMicrotask(() => this.finishAbort());
  }

  finishAbort(): void {
    if (this.finished) return;
    this.finished = true;
    this.dispatchEvent(new Event("abort"));
  }
}

export class FakeDatabase extends TrackedTarget {
  closing = false;
  closeCalls = 0;
  createCalls = 0;
  closeFails = false;
  throwRequest = false;
  storeExists = true;
  transactionError: string | undefined;
  readonly transactions: FakeTransaction[] = [];
  readonly objectStoreNames = {
    contains: (): boolean => this.storeExists,
  } as unknown as DOMStringList;

  constructor(readonly factory: FakeIndexedDb) {
    super();
  }

  createObjectStore(): IDBObjectStore {
    this.createCalls++;
    this.storeExists = true;
    return {} as IDBObjectStore;
  }

  transaction(): IDBTransaction {
    const error = this.closing ? "InvalidStateError" : this.transactionError;
    if (error) throw new DOMException("private database name", error);
    const transaction = new FakeTransaction(this);
    this.transactions.push(transaction);
    return transaction as unknown as IDBTransaction;
  }

  close(): void {
    this.closeCalls++;
    if (this.closeFails) throw new Error("private database name");
    this.closing = true;
  }
}

export class FakeIndexedDb {
  readonly databases: FakeDatabase[] = [];
  readonly requests: FakeRequest<IDBDatabase>[] = [];
  readonly targets: [string, number | undefined][] = [];
  readonly data = new Map<IDBValidKey, unknown>();
  autoOpen = true;
  autoTransactions = true;
  configure: (database: FakeDatabase) => void = (): void => {};

  open(name: string, version?: number): IDBOpenDBRequest {
    const request = new FakeRequest<IDBDatabase>();
    const database = new FakeDatabase(this);
    this.configure(database);
    this.targets.push([name, version]);
    this.requests.push(request);
    this.databases.push(database);
    request.result = database as unknown as IDBDatabase;
    if (this.autoOpen) queueMicrotask(() => request.succeed(request.result));
    return request as unknown as IDBOpenDBRequest;
  }
}

export async function microtasks(): Promise<void> {
  for (let tick = 0; tick < 16; tick++) await Promise.resolve();
}
