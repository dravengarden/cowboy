import {
  assertEquals,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert@1.0.19";
import {
  type KvStorage,
  persisted,
  type StorageChange,
  type StorageChanges,
} from "./store.ts";

const bool = {
  serialize: (value: boolean) => value ? "1" : "0",
  deserialize: (raw: string) => {
    if (raw !== "1" && raw !== "0") throw new Error("invalid boolean");
    return raw === "1";
  },
};

function fixture() {
  const data = new Map<string, string>();
  let reads = 0;
  let writes = 0;
  const storage: KvStorage = {
    getItem: (key) => {
      reads++;
      return data.get(key) ?? null;
    },
    setItem: (key, value) => {
      writes++;
      data.set(key, value);
    },
    removeItem: (key) => {
      data.delete(key);
    },
  };
  const listeners = new Set<(event: StorageChange) => void>();
  const retired: Array<(event: StorageChange) => void> = [];
  const changes: StorageChanges = {
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
        retired.push(listener);
      };
    },
  };
  const emit = (
    key: string | null = "key",
    storageArea: KvStorage | null = storage,
  ) => {
    for (const listener of [...listeners]) listener({ key, storageArea });
  };
  return {
    data,
    storage,
    changes,
    listeners,
    retired,
    emit,
    counts: () => ({ reads, writes }),
  };
}

Deno.test("persistence requires an explicit typed codec", () => {
  if (false) {
    // @ts-expect-error no default JSON.parse(raw) as T boundary
    persisted("key", false);
    // @ts-expect-error serializer alone does not validate untrusted input
    persisted("key", false, { serialize: String });
    // @ts-expect-error a codec cannot widen the inferred store type
    persisted("key", false, { ...bool, deserialize: () => "false" });
    // @ts-expect-error decoder must return the store's declared type
    persisted<boolean>("key", false, { ...bool, deserialize: () => "false" });
  }
});

Deno.test("listeners are owned by subscriptions, not import-time construction", () => {
  const f = fixture();
  const store = persisted("key", false, { ...bool, ...f });
  assertEquals(f.listeners.size, 0);
  let calls = 0;
  const listener = () => calls++;
  const offA = store.subscribe(listener);
  const offB = store.subscribe(listener);
  assertEquals(f.listeners.size, 1);
  store.set(true);
  assertEquals(calls, 2);
  offA();
  offA();
  assertEquals(f.listeners.size, 1);
  store.set(false);
  assertEquals(calls, 3);
  offB();
  assertEquals(f.listeners.size, 0);
  store.dispose();
  store.dispose();
  assertEquals(f.storage.getItem("key"), "0"); // cleanup is not deletion
});

Deno.test("storage area/key filter, clear, and stale events read current backend state", () => {
  const f = fixture();
  const store = persisted("key", false, { ...bool, ...f });
  let calls = 0;
  store.subscribe(() => calls++);
  f.data.set("key", "1");
  f.emit("unrelated");
  f.emit("key", fixture().storage);
  f.emit("key", null);
  assertEquals(store.get(), false);
  f.emit();
  f.emit();
  assertEquals(store.get(), true);
  assertEquals(calls, 1);
  f.data.clear();
  f.emit(null);
  assertEquals(store.get(), false);
  assertEquals(calls, 2);
  store.dispose();
});

Deno.test("detached reads and re-subscription refresh with stable object snapshots", () => {
  const f = fixture();
  const codec = {
    serialize: JSON.stringify,
    deserialize: (raw: string): { on: boolean } => {
      const value: unknown = JSON.parse(raw);
      if (
        !value || typeof value !== "object" || !("on" in value) ||
        typeof value.on !== "boolean"
      ) throw new Error("invalid object");
      return { on: value.on };
    },
  };
  f.data.set("key", '{"on":true}');
  const store = persisted("key", { on: false }, { ...codec, ...f });
  assertStrictEquals(store.get(), store.get());
  let calls = 0;
  const off = store.subscribe(() => calls++);
  f.data.set("key", '{"on":false}');
  f.emit();
  assertEquals(calls, 1);
  assertStrictEquals(store.get(), store.get());
  off();
  f.data.set("key", '{"on":true}');
  assertEquals(store.get().on, true);
  f.data.set("key", '{"on":false}');
  store.subscribe(() => calls++);
  assertEquals(store.get().on, false);
  assertEquals(calls, 2);
  store.dispose();
});

Deno.test("disposed and retired listener incarnations cannot publish into a replacement", () => {
  const f = fixture();
  const first = persisted("key", false, { ...bool, ...f });
  let calls = 0;
  const off = first.subscribe(() => calls++);
  off();
  first.subscribe(() => calls++);
  f.data.set("key", "1");
  f.retired[0]!({ key: "key", storageArea: f.storage });
  assertEquals(calls, 0); // even the same store's new subscription is fenced
  first.dispose();
  const second = persisted("key", false, { ...bool, ...f });
  second.subscribe(() => calls++);
  f.data.set("key", "0");
  for (const old of f.retired) old({ key: "key", storageArea: f.storage });
  assertEquals(calls, 0);
  assertEquals(first.get(), false);
  assertEquals(second.get(), true);
  assertThrows(() => first.set(true), Error, "disposed");
  assertThrows(() => first.subscribe(() => {}), Error, "disposed");
  second.dispose();
});

Deno.test("getItem/decode failures fall back without throwing or rewriting stored data", () => {
  const f = fixture();
  const issues: string[] = [];
  f.data.set("key", '{"not":"a boolean"}');
  const store = persisted("key", true, {
    ...bool,
    ...f,
    onError: (issue) => issues.push(issue),
  });
  assertEquals(store.get(), true);
  assertEquals(issues, ["decode"]); // unchanged corrupt bytes decode once
  assertEquals(f.counts().writes, 0);
  const broken = persisted("key", false, {
    ...bool,
    storage: {
      ...f.storage,
      getItem: () => {
        throw new Error("denied");
      },
    },
    onError: (issue) => issues.push(issue),
  });
  assertEquals(broken.get(), false);
  assertEquals(issues.slice(1), ["read", "read"]);
  store.dispose();
  broken.dispose();
});

Deno.test("quota and serializer failures still commit and notify coherent in-memory state", () => {
  for (const phase of ["write", "encode"] as const) {
    const f = fixture();
    f.data.set("key", "0");
    const issues: string[] = [];
    const storage = {
      ...f.storage,
      setItem: () => {
        throw new Error("quota");
      },
    };
    const store = persisted("key", false, {
      ...bool,
      storage,
      changes: f.changes,
      ...(phase === "encode"
        ? {
          serialize: () => {
            throw new Error("encode");
          },
        }
        : {}),
      onError: (issue) => issues.push(issue),
    });
    let observed = false;
    const off = store.subscribe(() => {
      observed = store.get();
    });
    store.set(true);
    assertEquals(observed, true);
    off();
    assertEquals(store.get(), true); // unchanged backend must not undo failed write
    assertEquals(issues, [phase]);
    assertEquals(f.data.get("key"), "0");
    store.dispose();
  }
});

Deno.test("explicit memory-only and crossTab:false never install event handlers", () => {
  const f = fixture();
  for (
    const options of [{ storage: null }, {
      storage: f.storage,
      crossTab: false,
    }]
  ) {
    const store = persisted("key", false, {
      ...bool,
      changes: f.changes,
      ...options,
    });
    store.subscribe(() => {});
    store.set((previous) => !previous);
    assertEquals(store.get(), true);
    assertEquals(f.listeners.size, 0);
    store.dispose();
  }
});

Deno.test("listener failures do not hide the committed state from other subscribers", () => {
  const f = fixture();
  const store = persisted("key", false, { ...bool, ...f });
  store.subscribe(() => {
    throw new Error("consumer");
  });
  let seen = false;
  store.subscribe(() => {
    seen = store.get();
  });
  assertThrows(() => store.set(true), AggregateError);
  assertEquals(seen, true);
  store.dispose();
});

Deno.test("disposal inside notification or updater fences remaining callbacks and writes", () => {
  const f = fixture();
  const store = persisted("key", false, { ...bool, ...f });
  store.subscribe(() => store.dispose());
  let stale = 0;
  store.subscribe(() => stale++);
  store.set(true);
  assertEquals(stale, 0);
  const other = persisted("other", false, { ...bool, ...f });
  assertThrows(
    () =>
      other.set(() => {
        other.dispose();
        return true;
      }),
    Error,
    "disposed",
  );
  assertEquals(f.data.has("other"), false);
});

Deno.test("default event adapter filters storage areas and handles clear without DOM-specific types", () => {
  const f = fixture();
  const store = persisted("key", false, { ...bool, storage: f.storage });
  let calls = 0;
  const off = store.subscribe(() => calls++);
  f.data.set("key", "1");
  globalThis.dispatchEvent(
    Object.assign(new Event("storage"), { key: "key", storageArea: null }),
  );
  assertEquals(store.get(), false);
  globalThis.dispatchEvent(
    Object.assign(new Event("storage"), { key: "key", storageArea: f.storage }),
  );
  assertEquals(store.get(), true);
  f.data.clear();
  globalThis.dispatchEvent(
    Object.assign(new Event("storage"), { key: null, storageArea: f.storage }),
  );
  assertEquals(store.get(), false);
  assertEquals(calls, 2);
  off();
  f.data.set("key", "1");
  globalThis.dispatchEvent(
    Object.assign(new Event("storage"), { key: "key", storageArea: f.storage }),
  );
  assertEquals(calls, 2);
  store.dispose();
});

Deno.test("cleanup failure is observable, idempotent and fences retained callbacks", () => {
  const f = fixture();
  const issues: string[] = [];
  let callback: ((event: StorageChange) => void) | undefined;
  const store = persisted("key", false, {
    ...bool,
    storage: f.storage,
    changes: {
      subscribe: (listener) => {
        callback = listener;
        return () => {
          throw new Error("cleanup");
        };
      },
    },
    onError: (phase) => {
      issues.push(phase);
      throw new Error("diagnostic");
    },
  });
  store.subscribe(() => {
    throw new Error("must not run");
  });
  store.dispose();
  store.dispose();
  f.data.set("key", "1");
  callback!({ key: "key", storageArea: f.storage });
  assertEquals(store.get(), false);
  assertEquals(issues, ["unlisten"]);
});

Deno.test("listener acquisition failure keeps local state usable and reports failure", () => {
  const f = fixture();
  const issues: string[] = [];
  const store = persisted("key", false, {
    ...bool,
    storage: f.storage,
    changes: {
      subscribe: () => {
        throw new Error("unsupported");
      },
    },
    onError: (phase) => issues.push(phase),
  });
  let calls = 0;
  const off = store.subscribe(() => calls++);
  store.set(true);
  assertEquals(calls, 1);
  assertEquals(issues, ["listen"]);
  off();
  store.dispose();
});
