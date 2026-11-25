import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import type { Attachment } from "./attachments.ts";

type Store = typeof import("./pendingEditStore.ts");

function image(id: string): Attachment {
  return {
    id,
    name: id + ".png",
    mimeType: "image/png",
    isImage: true,
    // Larger than the quota the tests give localStorage.
    block: { type: "image", data: "QUJD".repeat(500), mimeType: "image/png" },
  };
}

const TEXT = "    indented  edit ![shot](cowboy-att:a1) tail";
const PREFIX = "cowboy:pending-edit:";

/** A localStorage that refuses anything image-sized, like a full quota. */
function fakeStorage(quota = Infinity): Storage & { readonly records: Map<string, string> } {
  const records = new Map<string, string>();
  return {
    records,
    get length(): number {
      return records.size;
    },
    key: (index: number) => [...records.keys()][index] ?? null,
    getItem: (key: string) => records.get(key) ?? null,
    setItem: (key: string, value: string): void => {
      if (value.length > quota) throw new Error("QuotaExceededError");
      records.set(key, value);
    },
    removeItem: (key: string) => void records.delete(key),
    clear: () => records.clear(),
  };
}

/** Fake database round trips still running, across every fake database. */
let inFlight = 0;

function fakeDatabase() {
  const state = { records: new Map<string, unknown>(), ready: true };
  const tick = async (): Promise<void> => {
    inFlight += 1;
    try {
      await new Promise((resolve) => setTimeout(resolve, 2));
    } finally {
      inFlight -= 1;
    }
  };
  const admit = (): void => {
    if (!state.ready) throw new Error("Product dataset requires an authenticated principal");
  };
  return Object.assign(state, {
    cache: <T>(scope: { session?: string }) => ({
      load: async (): Promise<T | null> => {
        admit();
        await tick();
        return (state.records.get(scope.session!) ?? null) as T | null;
      },
      save: async (value: T): Promise<void> => {
        admit();
        await tick();
        state.records.set(scope.session!, value);
      },
      discard: async (): Promise<void> => {
        admit();
        await tick();
        state.records.delete(scope.session!);
      },
    }),
    cacheSessions: async (): Promise<string[]> => {
      admit();
      await tick();
      return [...state.records.keys()];
    },
  });
}

let generation = 0;
async function loadPage(storage: Storage): Promise<Store> {
  (globalThis as { localStorage?: Storage }).localStorage = storage;
  generation += 1;
  return await import("./pendingEditStore.ts?page=" + String(generation)) as Store;
}

/**
 * Wait until the store has no database round trip running for 30ms. A fixed
 * delay alone loses the race on a loaded host, where the fake's timers fire
 * late; the bound turns a store that never settles into a failed assertion.
 */
const settle = async (): Promise<void> => {
  for (let quiet = 0, waited = 0; quiet < 3 && waited < 5000; waited += 10) {
    await new Promise((resolve) => setTimeout(resolve, 10));
    quiet = inFlight === 0 ? quiet + 1 : 0;
  }
};
const edit = (session: string, text = TEXT, attachments = [image("a1")]) => ({
  sessionId: session,
  kind: "draft" as const,
  id: "row-1",
  text,
  attachments,
  baseText: "before",
  baseAttachments: [],
});

test("a row edit's images survive a reload that a full localStorage would have stripped", async () => {
  // Room for text, not for an image: the old store kept only the text here.
  const storage = fakeStorage(900);
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("quota"));
  assertEquals(first.pendingEditsDurable(), false);
  await settle();
  assertEquals(first.pendingEditsDurable(), true);
  const mirror = JSON.parse([...storage.records.values()][0]!);
  assertEquals(mirror.text, TEXT);
  assertEquals(mirror.attachments, []);
  assertEquals(mirror.attachmentsInDatabase, true);

  // The reloaded page reads the database before its account is bound.
  database.ready = false;
  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  assertEquals(second.getPendingEdit("quota", "draft", "row-1")?.text, TEXT);
  // The row editor reopens at once and writes its seed back, without bytes.
  second.setPendingEdit(edit("quota", TEXT + " more", []));
  second.flushPendingEdits();
  // Not claimable for parked-draft recovery without its images.
  assertEquals(second.claimOrphanedPendingEdits("quota", new Set(), new Set()), []);

  const arrived: string[][] = [];
  second.subscribePendingEditRestore("quota", "draft", "row-1", (restored) => {
    arrived.push(restored.attachments.map((attachment) => attachment.id));
  });
  database.ready = true;
  second.retryPendingEditRestore();
  await settle();
  const recovered = second.getPendingEdit("quota", "draft", "row-1");
  assertEquals(recovered?.text, TEXT + " more");
  assertEquals(recovered?.attachments.map((attachment) => attachment.id), ["a1"]);
  assertEquals(arrived, [["a1"]]);
  assertEquals(
    second.claimOrphanedPendingEdits("quota", new Set(), new Set()).length,
    1,
  );
});

test("finishing an edit removes its bytes, and a later text-only edit writes none", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  const page = await loadPage(storage);
  page.attachPendingEditDatabase(database as never);
  await settle();
  page.setPendingEdit(edit("done"));
  page.flushPendingEdits();
  await settle();
  assertEquals(database.records.has("done"), true);
  page.clearPendingEdit("done", "draft", "row-1");
  await settle();
  assertEquals(database.records.has("done"), false);
  assertEquals([...storage.records.keys()].filter((key) => key.startsWith(PREFIX)), []);

  page.setPendingEdit(edit("done", "plain  text", []));
  page.flushPendingEdits();
  await settle();
  assertEquals(database.records.has("done"), false);
  assertEquals(JSON.parse([...storage.records.values()][0]!).text, "plain  text");
});

test("one session record keeps every edit's images apart", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("pair"));
  first.setPendingEdit({
    ...edit("pair", "queued ![b](cowboy-att:b1)", [image("b1")]),
    kind: "queued",
    id: "row-2",
  });
  first.flushPendingEdits();
  await settle();
  // Removing the image from one edit leaves the other's bytes in the record.
  first.setPendingEdit(edit("pair", "    indented  edit tail", []));
  first.flushPendingEdits();
  await settle();

  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  assertEquals(second.getPendingEdit("pair", "draft", "row-1")?.attachments, []);
  assertEquals(second.getPendingEdit("pair", "draft", "row-1")?.text, "    indented  edit tail");
  assertEquals(
    second.getPendingEdit("pair", "queued", "row-2")?.attachments.map((a) => a.id),
    ["b1"],
  );
});

test("without a database the edit still keeps its text", async () => {
  const storage = fakeStorage(900);
  const first = await loadPage(storage);
  first.setPendingEdit(edit("legacy"));
  first.flushPendingEdits();
  const second = await loadPage(storage);
  assertEquals(
    second.getPendingEdit("legacy", "draft", "row-1")?.text,
    "    indented  edit tail",
  );
});

test("an edit another tab already recovered is not recovered again", async () => {
  // Tabs of one origin share localStorage; each holds its own copy in memory.
  const storage = fakeStorage();
  const first = await loadPage(storage);
  first.setPendingEdit({ ...edit("tabs", "typed once", []), kind: "queued" });
  first.flushPendingEdits();
  const second = await loadPage(storage);
  assertEquals(second.getPendingEdit("tabs", "queued", "row-1")?.text, "typed once");

  const claimed = first.claimOrphanedPendingEdits("tabs", new Set(), new Set());
  assertEquals(claimed.length, 1);
  first.finishOrphanedPendingEdit(claimed[0]!, true);
  // The user may already have sent or deleted that draft: the second tab must
  // not park it once more when its own queue patch arrives later.
  assertEquals(second.claimOrphanedPendingEdits("tabs", new Set(), new Set()), []);
  assertEquals(second.getPendingEdit("tabs", "queued", "row-1"), null);

  // Typing this tab has not flushed yet is its own and still recovers.
  second.setPendingEdit({ ...edit("tabs", "typed here", []), kind: "queued" });
  assertEquals(second.claimOrphanedPendingEdits("tabs", new Set(), new Set()).length, 1);
});

test("an image still encoding settles into the edit after its editor is gone", async () => {
  const page = await loadPage(fakeStorage());
  const placeholder = { ...image("p1"), block: { type: "image" as const, data: "", mimeType: "image/png" }, pending: true };
  const settledSessions: string[] = [];
  const stop = page.onPendingEditEncodesSettled((session) => settledSessions.push(session));
  const settleImage = page.beginPendingEditEncoding("encode", "queued", "row-1", [placeholder]);
  const text = "caption ![shot](cowboy-att:p1)";
  page.setPendingEdit({ ...edit("encode", text, [placeholder]), kind: "queued" });
  // The placeholder keeps its token, and recovery waits for its bytes.
  assertEquals(page.getPendingEdit("encode", "queued", "row-1")?.text, text);
  assertEquals(page.claimOrphanedPendingEdits("encode", new Set(), new Set()), []);

  // The row was sent elsewhere and its editor unmounted; the encode finishes.
  settleImage([image("p1")]);
  assertEquals(settledSessions, ["encode"]);
  const claimed = page.claimOrphanedPendingEdits("encode", new Set(), new Set());
  assertEquals(claimed.length, 1);
  assertEquals(claimed[0]?.text, text);
  assertEquals(claimed[0]?.attachments.map((attachment) => attachment.id), ["p1"]);
  page.finishOrphanedPendingEdit(claimed[0]!, true);

  // A failed encode drops its token instead of parking a broken image.
  const settleFailure = page.beginPendingEditEncoding("encode", "queued", "row-2", [placeholder]);
  page.setPendingEdit({ ...edit("encode", text, [placeholder]), kind: "queued", id: "row-2" });
  settleFailure([]);
  assertEquals(page.getPendingEdit("encode", "queued", "row-2")?.text.trimEnd(), "caption");

  // A picked file gets its token only once encoded; the record places it.
  const settleFile = page.beginPendingEditEncoding("encode", "queued", "row-3", []);
  page.setPendingEdit({ ...edit("encode", "file", []), kind: "queued", id: "row-3" });
  settleFile([image("f1")]);
  const file = page.getPendingEdit("encode", "queued", "row-3");
  assertEquals(file?.attachments.map((attachment) => attachment.id), ["f1"]);
  assertEquals(file?.text.includes("cowboy-att:f1"), true);
  stop();
});

test("discarding an older copy keeps another tab's newer edit", async () => {
  const storage = fakeStorage();
  const first = await loadPage(storage);
  first.setPendingEdit({ ...edit("newer", "older text", []), kind: "queued" });
  first.flushPendingEdits();
  const second = await loadPage(storage);
  await new Promise((resolve) => setTimeout(resolve, 5));
  second.setPendingEdit({ ...edit("newer", "newer text", []), kind: "queued" });
  second.flushPendingEdits();

  first.clearPendingEdit("newer", "queued", "row-1");
  const claimed = second.claimOrphanedPendingEdits("newer", new Set(), new Set());
  assertEquals(claimed.map((record) => record.text), ["newer text"]);
});

test("a file whose edit was discarded does not join a later edit of the row", async () => {
  const page = await loadPage(fakeStorage());
  const settleFile = page.beginPendingEditEncoding("discard", "queued", "row-1", []);
  page.setPendingEdit({ ...edit("discard", "first", []), kind: "queued" });
  assertEquals(page.pendingEditEncoding("discard", "queued", "row-1"), true);
  page.clearPendingEdit("discard", "queued", "row-1");
  assertEquals(page.pendingEditEncoding("discard", "queued", "row-1"), false);
  page.setPendingEdit({ ...edit("discard", "second", []), kind: "queued" });
  settleFile([image("f1")]);
  const later = page.getPendingEdit("discard", "queued", "row-1");
  assertEquals(later?.text, "second");
  assertEquals(later?.attachments, []);
});

test("an editor reopened during an encode adopts the settled bytes", async () => {
  const page = await loadPage(fakeStorage());
  const placeholder = { ...image("p9"), block: { type: "image" as const, data: "", mimeType: "image/png" }, pending: true };
  const settle = page.beginPendingEditEncoding("reopen", "queued", "row-1", [placeholder]);
  page.setPendingEdit({ ...edit("reopen", "x ![s](cowboy-att:p9)", [placeholder]), kind: "queued" });
  const adopted: string[][] = [];
  page.subscribePendingEditRestore("reopen", "queued", "row-1", (restored) => {
    adopted.push(restored.attachments.map((attachment) => attachment.id));
  });
  settle([image("p9")]);
  assertEquals(adopted, [["p9"]]);
});

test("an unchanged rerender does not overwrite another tab's newer mirror", async () => {
  const storage = fakeStorage();
  const first = await loadPage(storage);
  first.setPendingEdit({ ...edit("rerender", "older", []), kind: "queued" });
  first.flushPendingEdits();
  const second = await loadPage(storage);
  await new Promise((resolve) => setTimeout(resolve, 5));
  second.setPendingEdit({ ...edit("rerender", "newer", []), kind: "queued" });
  second.flushPendingEdits();
  // A queue patch makes the first tab write its unchanged buffer again.
  first.setPendingEdit({ ...edit("rerender", "older", []), kind: "queued", baseText: "patched" });
  first.flushPendingEdits();
  const mirror = JSON.parse([...storage.records.values()][0]!);
  assertEquals(mirror.text, "newer");
  first.clearPendingEdit("rerender", "queued", "row-1");
  assertEquals(
    second.claimOrphanedPendingEdits("rerender", new Set(), new Set()).map((record) => record.text),
    ["newer"],
  );
});

test("an image pasted while the edit's bytes load keeps its token and settles", async () => {
  const storage = fakeStorage(900);
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("paste"));
  first.flushPendingEdits();
  await settle();

  database.ready = false;
  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  // Only this test's session: an earlier test's page may still flush its
  // debounced mirror into the shared localStorage this page hydrates from.
  const settledSessions: string[] = [];
  const stop = second.onPendingEditEncodesSettled((session) => {
    if (session === "paste") settledSessions.push(session);
  });
  // The editor reopens on the edit and the user pastes before a1 is back.
  const placeholder = {
    ...image("p1"),
    block: { type: "image" as const, data: "", mimeType: "image/png" },
    pending: true,
  };
  const settleImage = second.beginPendingEditEncoding("paste", "draft", "row-1", [placeholder]);
  const pasted = TEXT + " ![new](cowboy-att:p1)";
  second.setPendingEdit(edit("paste", pasted, [placeholder]));
  second.flushPendingEdits();

  // The database answers first: the paste's token is not an orphan.
  database.ready = true;
  second.retryPendingEditRestore();
  await settle();
  assertEquals(second.getPendingEdit("paste", "draft", "row-1")?.text, pasted);
  // Recovery is told, but the claim waits for the encode.
  assertEquals(settledSessions, ["paste"]);
  assertEquals(second.claimOrphanedPendingEdits("paste", new Set(), new Set()), []);

  settleImage([image("p1")]);
  const recovered = second.getPendingEdit("paste", "draft", "row-1");
  assertEquals(recovered?.text, pasted);
  assertEquals(recovered?.attachments.map((attachment) => attachment.id), ["a1", "p1"]);
  assertEquals(settledSessions, ["paste", "paste"]);
  stop();
});

test("orphan recovery is rechecked once an edit's bytes arrive", async () => {
  const storage = fakeStorage(900);
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("arrive"));
  first.flushPendingEdits();
  await settle();

  database.ready = false;
  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  const settledSessions: string[] = [];
  const stop = second.onPendingEditEncodesSettled((session) => {
    if (session === "arrive") settledSessions.push(session);
  });
  assertEquals(second.claimOrphanedPendingEdits("arrive", new Set(), new Set()), []);
  database.ready = true;
  second.retryPendingEditRestore();
  await settle();
  assertEquals(settledSessions, ["arrive"]);
  assertEquals(second.claimOrphanedPendingEdits("arrive", new Set(), new Set()).length, 1);
  stop();
});

test("an edit another tab retired while its bytes loaded is not written back", async () => {
  const storage = fakeStorage(900);
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("retired"));
  first.flushPendingEdits();
  await settle();

  database.ready = false;
  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  // The other tab saved or discarded the edit; its storage event is late.
  for (const key of [...storage.records.keys()]) {
    if (key.startsWith(PREFIX)) storage.records.delete(key);
  }
  database.ready = true;
  second.retryPendingEditRestore();
  await settle();
  second.flushPendingEdits();
  assertEquals(second.getPendingEdit("retired", "draft", "row-1"), null);
  // Earlier tests' pages may still flush their own mirrors into this storage.
  assertEquals([...storage.records.keys()].filter((key) => key.includes('"retired"')), []);
  assertEquals(second.claimOrphanedPendingEdits("retired", new Set(), new Set()), []);
});

const slots = (database: ReturnType<typeof fakeDatabase>, session: string): string[] =>
  Object.keys(
    (database.records.get(session) as { edits?: Record<string, unknown> } | undefined)?.edits ?? {},
  ).sort();

test("tabs that each know only their own edit never erase the other's bytes", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  // Each tab starts its edit after the other loaded, so neither sees it.
  second.setPendingEdit({
    ...edit("tabs", "queued ![b](cowboy-att:b1)", [image("b1")]),
    kind: "queued",
    id: "row-2",
  });
  second.flushPendingEdits();
  await settle();
  first.setPendingEdit(edit("tabs"));
  first.flushPendingEdits();
  await settle();
  assertEquals(slots(database, "tabs"), ["draft:row-1", "queued:row-2"]);
  // Ending its own edit takes out only that slot.
  first.clearPendingEdit("tabs", "draft", "row-1");
  await settle();
  assertEquals(slots(database, "tabs"), ["queued:row-2"]);
  const third = await loadPage(storage);
  third.attachPendingEditDatabase(database as never);
  await settle();
  assertEquals(
    third.getPendingEdit("tabs", "queued", "row-2")?.attachments.map((a) => a.id),
    ["b1"],
  );
});

test("a stale copy of another tab's edit never overwrites its newer bytes", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("stale"));
  first.flushPendingEdits();
  await settle();
  const second = await loadPage(storage);
  second.attachPendingEditDatabase(database as never);
  await settle();
  // The first tab replaces the image; the second still holds the old one.
  await new Promise((resolve) => setTimeout(resolve, 5));
  first.setPendingEdit(edit("stale", "new ![n](cowboy-att:n1)", [image("n1")]));
  first.flushPendingEdits();
  await settle();
  // A write of the second tab for another edit of the session.
  second.setPendingEdit({
    ...edit("stale", "other ![o](cowboy-att:o1)", [image("o1")]),
    id: "row-9",
  });
  second.flushPendingEdits();
  await settle();
  const stored = database.records.get("stale") as {
    edits: Record<string, { id: string }[]>;
  };
  assertEquals(stored.edits["draft:row-1"]?.map((a) => a.id), ["n1"]);
  assertEquals(stored.edits["draft:row-9"]?.map((a) => a.id), ["o1"]);
});

test("bytes no tab mirrors any more are swept on the next write", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  database.records.set("sweep", {
    edits: { "draft:gone": [image("g1")] },
    savedAt: 1,
  });
  const first = await loadPage(storage);
  first.attachPendingEditDatabase(database as never);
  await settle();
  first.setPendingEdit(edit("sweep"));
  first.flushPendingEdits();
  await settle();
  assertEquals(slots(database, "sweep"), ["draft:row-1"]);
});
