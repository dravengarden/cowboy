import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { Attachment } from "./attachments.ts";
import type { StoredDraft } from "./draftRestore.ts";

type DraftStore = typeof import("./draftStore.ts");

function image(id: string): Attachment {
  return {
    id,
    name: `${id}.png`,
    mimeType: "image/png",
    isImage: true,
    block: { type: "image", data: "cGl4ZWw=", mimeType: "image/png" },
  };
}

const KEY = "cowboy:composer-draft:";
const TEXT = "左右边框有点 ![shot](cowboy-att:a1) 更多文字";

function fakeStorage(): Storage & { readonly records: Map<string, string> } {
  const records = new Map<string, string>();
  return {
    records,
    get length(): number {
      return records.size;
    },
    key: (index: number) => [...records.keys()][index] ?? null,
    getItem: (key: string) => records.get(key) ?? null,
    setItem: (key: string, value: string) => void records.set(key, value),
    removeItem: (key: string) => void records.delete(key),
    clear: () => records.clear(),
  };
}

interface Database {
  readonly records: Map<string, StoredDraft>;
  ready: boolean;
  /** Writes are accepted but never reach the disk: the page died first. */
  dropWrites: boolean;
  loads: number;
}

function fakeDatabase(): Database & Parameters<DraftStore["attachDraftDatabase"]>[0] {
  const state: Database = { records: new Map(), ready: true, dropWrites: false, loads: 0 };
  const tick = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 2));
  const admit = (): void => {
    if (!state.ready) throw new Error("Product dataset requires an authenticated principal");
  };
  return Object.assign(state, {
    cache: <T>(scope: { kind: string; session?: string }) => ({
      load: async (): Promise<T | null> => {
        admit();
        await tick();
        state.loads += 1;
        return (state.records.get(scope.session!) ?? null) as T | null;
      },
      save: async (value: T): Promise<void> => {
        admit();
        await tick();
        await tick();
        if (!state.dropWrites) state.records.set(scope.session!, value as StoredDraft);
      },
      discard: async (): Promise<void> => {
        admit();
        await tick();
        if (!state.dropWrites) state.records.delete(scope.session!);
      },
    }),
    cacheSessions: async (): Promise<string[]> => {
      admit();
      await tick();
      return [...state.records.keys()];
    },
  }) as Database & Parameters<DraftStore["attachDraftDatabase"]>[0];
}

let generation = 0;
/** A page load: the module is evaluated again over the same storage. */
async function loadPage(storage: Storage): Promise<DraftStore> {
  (globalThis as { localStorage?: Storage }).localStorage = storage;
  generation += 1;
  return await import(`./draftStore.ts?page=${String(generation)}`) as DraftStore;
}

const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 30));
const leavePage = (): void => void globalThis.dispatchEvent(new Event("pagehide"));
const mirror = (storage: { records: Map<string, string> }, session: string): unknown =>
  JSON.parse(storage.records.get(KEY + session) ?? "null");

async function writeImageDraft(
  storage: Storage,
  database: ReturnType<typeof fakeDatabase>,
  session: string,
): Promise<void> {
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  page.setDraft(session, { text: TEXT, attachments: [image("a1")] });
  await settle();
  leavePage();
}

test("a pasted image survives a reload that reads the database before the account is bound", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  await writeImageDraft(storage, database, "early");

  // The reloaded page evaluates the store before its account is bound.
  database.ready = false;
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  assertEquals(page.getDraft("early").text, TEXT);

  // The composer mounts meanwhile and writes its seed back, as it always does.
  const seed = page.getDraft("early");
  page.setDraft("early", { text: seed.text, attachments: seed.attachments });
  leavePage();
  assertEquals(mirror(storage, "early"), {
    text: TEXT,
    attachments: [],
    attachmentsInDatabase: true,
  });
  assertEquals(database.records.has("early"), true);

  const restored: string[][] = [];
  page.subscribeDraftRestore("early", (draft) => {
    restored.push(draft.attachments.map((attachment) => attachment.id));
  });
  database.ready = true;
  page.retryDraftRestore();
  await settle();
  assertEquals(page.getDraft("early").text, TEXT);
  assertEquals(page.getDraft("early").attachments.map((attachment) => attachment.id), ["a1"]);
  assertEquals(restored, [["a1"]]);
});

test("text typed while the bytes are still promised keeps its image", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  await writeImageDraft(storage, database, "typing");

  database.ready = false;
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  page.setDraft("typing", { text: `${TEXT} 继续写`, attachments: [] });
  database.ready = true;
  page.retryDraftRestore();
  await settle();
  assertEquals(page.getDraft("typing").text, `${TEXT} 继续写`);
  assertEquals(page.getDraft("typing").attachments.length, 1);
  leavePage();
  await settle();
  assertEquals(database.records.get("typing")?.text, `${TEXT} 继续写`);
});

test("an image deleted while its bytes were promised does not come back", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  await writeImageDraft(storage, database, "deleted");

  database.ready = false;
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  page.setDraft("deleted", { text: "左右边框有点 更多文字", attachments: [] });
  database.ready = true;
  page.retryDraftRestore();
  await settle();
  assertEquals(page.getDraft("deleted"), { text: "左右边框有点 更多文字", attachments: [] });
});

test("a record that really is gone drops only its token", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  storage.setItem(
    KEY + "gone",
    JSON.stringify({ text: `    indented  ${TEXT}`, attachments: [], attachmentsInDatabase: true }),
  );
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  assertEquals(page.getDraft("gone").text, "    indented  左右边框有点 更多文字");
});

test("a draft sent before the database answered is not brought back by its record", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  await writeImageDraft(storage, database, "sent");

  database.ready = false;
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  page.setDraft("sent", { text: "", attachments: [] });
  assertEquals(storage.records.has(KEY + "sent"), false);
  database.ready = true;
  page.retryDraftRestore();
  await settle();
  assertEquals(page.getDraft("sent"), { text: "", attachments: [] });
  assertEquals(database.records.has("sent"), false);

  const next = await loadPage(storage);
  next.attachDraftDatabase(database);
  await settle();
  assertEquals(next.getDraft("sent"), { text: "", attachments: [] });
});

test("a save cannot land after the discard that followed it", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  page.setDraft("ordered", { text: TEXT, attachments: [image("a1")] });
  page.setDraft("ordered", { text: "", attachments: [] });
  await settle();
  assertEquals(database.records.has("ordered"), false);
});

test("new attachment bytes start their write at once and hold a reload until they land", async () => {
  const storage = fakeStorage();
  const database = fakeDatabase();
  const page = await loadPage(storage);
  page.attachDraftDatabase(database);
  await settle();
  page.setDraft("bytes", { text: TEXT, attachments: [image("a1")] });
  // No typing pause has elapsed, yet the mirror is written and the bytes are
  // on their way.
  assertEquals(mirror(storage, "bytes"), {
    text: TEXT,
    attachments: [],
    attachmentsInDatabase: true,
  });
  assertEquals(page.draftsDurable(), false);
  await settle();
  assertEquals(page.draftsDurable(), true);
  assertEquals(database.records.get("bytes")?.attachments.length, 1);
});

test("a reload leaves a text draft exactly as typed", async () => {
  const storage = fakeStorage();
  const text = "def f():\n    return  1  # two  spaces\n\n\ttabbed";
  const first = await loadPage(storage);
  first.setDraft("code", { text, attachments: [] });
  leavePage();
  const second = await loadPage(storage);
  assertEquals(second.getDraft("code").text, text);
});
