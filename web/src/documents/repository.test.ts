import { test } from "bun:test";
import { assertEquals, assertRejects } from "@std/assert";
import type { ClientSnapshot, LocalPersistence } from "@cowboy/state-sync";
import { createDraftRepository } from "./repository.ts";
import {
  type DraftDocument,
  type DraftMutationArgs,
  expectedRevision,
  projectDraft,
} from "./model.ts";

function fixture() {
  const local = new Map<string, ClientSnapshot<DraftDocument | null>>();
  const server = new Map<string, DraftDocument>();
  const operations = new Map<string, string>();
  const history: DraftDocument[] = [];
  const notices: string[] = [];
  const requests: string[] = [];
  const lifetime = new AbortController();
  let blocked = false;
  let offline = false;
  let gateway = false;
  let dropReply = false;
  let saving: (() => Promise<void>) | undefined;
  let index: unknown = null;
  const client = (
    local: Map<string, ClientSnapshot<DraftDocument | null>>,
    offline: () => boolean,
  ) =>
    createDraftRepository({
      signal: lifetime.signal,
      persistence: (
        id,
      ): LocalPersistence<ClientSnapshot<DraftDocument | null>> => ({
        load: () => Promise.resolve(local.get(id) ?? null),
        save: async (snapshot) => {
          if (blocked) throw new Error("quota");
          await saving?.();
          local.set(id, structuredClone(snapshot));
        },
      }),
      cache: {
        load: () => Promise.resolve(index as never),
        save: (value) => {
          index = value;
          return Promise.resolve();
        },
        discard: () => Promise.resolve(),
      },
      localIds: () => Promise.resolve([...local.keys()]),
      notify: (message) => notices.push(message),
      request: (path, init) => {
        if (offline()) return Promise.reject(new Error("offline"));
        requests.push(path);
        if (path === "/api/drafts") {
          return Promise.resolve(
            Response.json({ entries: [...server.values()] }),
          );
        }
        if (path === "/api/drafts/mutations") {
          // A proxy in front of a restarting controller: no JSON body.
          if (gateway) {
            return Promise.resolve(new Response("", { status: 502 }));
          }
          const args = JSON.parse(String(init?.body)) as DraftMutationArgs & {
            operation_id: string;
          };
          const current = server.get(args.document_id) ?? null;
          if (
            args.change.type === "write" &&
            args.change.body.includes("OVERSIZE")
          ) {
            return Promise.resolve(
              Response.json({ error: "Too large" }, { status: 422 }),
            );
          }
          if (operations.has(args.operation_id)) {
            return Promise.resolve(Response.json(current));
          }
          if (
            args.change.type !== "create" &&
            (!current ||
              expectedRevision(current, args.change) !== args.expected_revision)
          ) {
            return Promise.resolve(
              Response.json({ current, error: "Conflict" }, { status: 409 }),
            );
          }
          const next = projectDraft(current, {
            ...args,
            authored_at_ms: Date.now(),
          })!;
          server.set(next.id, next);
          operations.set(args.operation_id, next.id);
          if (dropReply) {
            dropReply = false;
            return Promise.reject(new Error("timeout"));
          }
          return Promise.resolve(Response.json(next));
        }
        if (path.endsWith("/history")) {
          return Promise.resolve(Response.json(history));
        }
        const row = server.get(path.split("/").at(-1)!);
        return Promise.resolve(
          row ? Response.json(row) : Response.json({}, { status: 404 }),
        );
      },
    });
  const create = () => client(local, () => offline);
  return {
    create,
    /** Another device of the same account: its own storage and connection. */
    device: () => {
      let disconnected = false;
      return {
        repository: client(new Map(), () => disconnected),
        offline: (value: boolean) => {
          disconnected = value;
        },
      };
    },
    local,
    server,
    requests,
    operations,
    history,
    notices,
    block: () => {
      blocked = true;
    },
    offline: (value: boolean) => {
      offline = value;
    },
    gateway: (value: boolean) => {
      gateway = value;
    },
    cacheIndex: (entries: readonly DraftDocument[]) => {
      index = entries.map(({ body: _b, attachments: _a, ...entry }) => entry);
    },
    /** The next accepted mutation is applied, but its reply never arrives. */
    dropReply: () => {
      dropReply = true;
    },
    /** Runs inside every local save, before it becomes durable. */
    whileSaving: (hook: (() => Promise<void>) | undefined) => {
      saving = hook;
    },
  };
}
async function settle(check: () => boolean): Promise<void> {
  for (let i = 0; i < 100; i++) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
  throw new Error("Draft owner did not settle");
}

test("storage failure never sends authored content", async () => {
  const f = fixture();
  const repository = f.create();
  f.block();
  await assertRejects(() =>
    repository.create("Private text", null, "document", "Unsaved")
  );
  assertEquals(f.requests, []);
  assertEquals(f.server.size, 0);
  await repository.dispose().catch(() => undefined);
});

test("offline create and edits survive reload and replay exactly once", async () => {
  const f = fixture();
  f.offline(true);
  const first = f.create();
  const id = await first.create("离线文档", null, "document", "initial");
  await first.document(id).change({
    type: "write",
    body: "离线编辑 📝",
    attachments: [],
  });
  assertEquals(f.server.size, 0);
  assertEquals(first.document(id).get().document?.body, "离线编辑 📝");
  await first.dispose();
  f.offline(false);
  const second = f.create();
  await second.start();
  await settle(() => second.document(id).get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "离线编辑 📝");
  assertEquals(f.server.get(id)?.body_revision, 2);
  await second.document(id).retry();
  assertEquals(f.operations.size, 2);
  await second.dispose();
});

test("an ancestor-less competing write is kept as a copy and never blocks the document", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "base");
  await settle(() => first.document(id).get().phase === "saved");
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "other device",
    revision: 2,
    body_revision: 2,
  });
  await first.document(id).refresh();
  // An outbox write queued by an older client: a stale revision, no base.
  await first.document(id).change({
    type: "write",
    body: "my unsaved typing",
    attachments: [],
  }, 1);
  await settle(() =>
    first.document(id).get().phase === "saved" &&
    [...f.server.values()].some((d) => d.body === "my unsaved typing")
  );
  assertEquals(f.server.get(id)?.body, "other device");
  assertEquals(first.document(id).get().document?.body, "other device");
  const copy = [...f.server.values()].find((d) => d.id !== id)!;
  assertEquals(copy.title, "Draft (conflicted copy)");
  assertEquals(f.notices.length, 1);
  // Later edits save normally.
  await first.document(id).change({
    type: "write",
    body: "other device, continued",
    attachments: [],
  });
  await settle(() => f.server.get(id)?.body === "other device, continued");
  await first.dispose();
});

test("an ancestor-less write merges through the server's recovery history", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "alpha beta");
  await settle(() => first.document(id).get().phase === "saved");
  f.history.push(f.server.get(id)!);
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "alpha beta gamma",
    revision: 2,
    body_revision: 2,
  });
  await first.document(id).change({
    type: "write",
    body: "ALPHA beta",
    attachments: [],
  }, 1);
  await settle(() => f.server.get(id)?.body === "ALPHA beta gamma");
  assertEquals(f.server.size, 1);
  assertEquals(f.notices, []);
  await first.dispose();
});

test("a write refused by a newer server text merges both edits and resends", async () => {
  const f = fixture();
  const phone = f.create();
  const id = await phone.create(
    "Draft",
    null,
    "document",
    "first line\nsecond line\n",
  );
  await settle(() => phone.document(id).get().phase === "saved");
  // Another device appended while this one was offline.
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "first line\nsecond line\nfrom desktop\n",
    revision: 2,
    body_revision: 2,
  });
  f.offline(true);
  await phone.document(id).change({
    type: "write",
    body: "first line edited\nsecond line\n",
    attachments: [],
  });
  await phone.document(id).change({
    type: "write",
    body: "first line edited twice\nsecond line\n",
    attachments: [],
  });
  f.offline(false);
  await phone.document(id).retry();
  await settle(() =>
    phone.document(id).get().phase === "saved" &&
    f.server.get(id)?.body_revision === 3
  );
  assertEquals(
    f.server.get(id)?.body,
    "first line edited twice\nsecond line\nfrom desktop\n",
  );
  assertEquals(phone.document(id).get().document?.body, f.server.get(id)?.body);
  await phone.dispose();
});

test("an editor writing against stale content merges with the newer replica", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "alpha beta");
  await settle(() => first.document(id).get().phase === "saved");
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "alpha beta gamma",
    revision: 2,
    body_revision: 2,
  });
  await first.document(id).refresh();
  await first.document(id).change(
    { type: "write", body: "ALPHA beta", attachments: [] },
    undefined,
    { body: "alpha beta", attachments: [] },
  );
  await settle(() => first.document(id).get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "ALPHA beta gamma");
  await first.dispose();
});

test("a refused rename is reapplied over the newer metadata", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Old", null, "document", "body");
  await settle(() => first.document(id).get().phase === "saved");
  f.server.set(id, {
    ...f.server.get(id)!,
    title: "Elsewhere",
    revision: 2,
    metadata_revision: 2,
  });
  await first.document(id).change({ type: "rename", title: "Mine" }, 1);
  await settle(() =>
    first.document(id).get().phase === "saved" &&
    f.server.get(id)?.title === "Mine"
  );
  assertEquals(f.server.get(id)?.metadata_revision, 3);
  await first.dispose();
});

test("a pushed revision refreshes only an open document that lacks it", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "body");
  await settle(() => first.document(id).get().phase === "saved");
  await first.start();
  const before = f.requests.filter((p) => p === `/api/drafts/${id}`).length;
  first.announce({ [id]: { ...f.server.get(id)!, body: undefined } });
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "pushed",
    revision: 2,
    body_revision: 2,
  });
  first.announce({
    [id]: {
      id,
      kind: "document",
      title: "Draft",
      parent_id: null,
      revision: 2,
      body_revision: 2,
      metadata_revision: 1,
      updated_at_ms: 1,
      deleted: false,
    },
  });
  await settle(() => first.document(id).get().document?.body === "pushed");
  assertEquals(
    f.requests.filter((p) => p === `/api/drafts/${id}`).length,
    before + 1,
  );
  assertEquals(
    first.get().entries.find((e) => e.id === id)?.revision,
    2,
  );
  await first.dispose();
});

test("merging a refused write never shows an older text than the local one", async () => {
  const f = fixture();
  const phone = f.create();
  const id = await phone.create("Draft", null, "document", "one\ntwo\n");
  const owner = phone.document(id);
  await settle(() => owner.get().phase === "saved");
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "one\ntwo\nthree\n",
    revision: 2,
    body_revision: 2,
  });
  const shown: string[] = [];
  owner.subscribe(() => shown.push(owner.get().document?.body ?? ""));
  await owner.change({ type: "write", body: "ONE\ntwo\n", attachments: [] });
  await settle(() =>
    owner.get().phase === "saved" &&
    f.server.get(id)?.body === "ONE\ntwo\nthree\n"
  );
  // Neither the pre-edit text nor the remote text without the local edit: an
  // open editor would fold either one in as if another device had typed it.
  assertEquals(shown.filter((body) => !body.startsWith("ONE")), []);
  await phone.dispose();
});

test("a write authored while a refused write is merged survives", async () => {
  const f = fixture();
  const phone = f.create();
  const id = await phone.create("Draft", null, "document", "one\ntwo\n");
  const owner = phone.document(id);
  await settle(() => owner.get().phase === "saved");
  f.server.set(id, {
    ...f.server.get(id)!,
    body: "one\ntwo\nthree\n",
    revision: 2,
    body_revision: 2,
  });
  await owner.change({ type: "write", body: "ONE\ntwo\n", attachments: [] });
  // The next local save is the merge of the refused write. The editor's
  // autosave lands exactly then, authored against its last written text.
  let typed: Promise<void> | undefined;
  f.whileSaving(async () => {
    if (typed) return;
    typed = owner.change(
      { type: "write", body: "ONE\ntwo\nextra\n", attachments: [] },
      undefined,
      { body: "ONE\ntwo\n", attachments: [] },
    );
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
  await settle(() => typed !== undefined);
  await typed;
  f.whileSaving(undefined);
  await settle(() =>
    owner.get().phase === "saved" &&
    owner.get().document?.body === f.server.get(id)?.body
  );
  assertEquals(f.server.get(id)?.body, "ONE\ntwo\nthree\nextra\n");
  assertEquals(f.server.size, 1);
  assertEquals(f.notices, []);
  await phone.dispose();
});

test("a gateway reply without JSON keeps the write queued instead of failing it", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "base");
  const owner = first.document(id);
  await settle(() => owner.get().phase === "saved");
  f.gateway(true);
  await owner.change({ type: "write", body: "typed", attachments: [] });
  await settle(() => owner.get().phase !== "saving");
  assertEquals(owner.get().phase, "local");
  assertEquals(owner.get().document?.body, "typed");
  f.gateway(false);
  await owner.retry();
  await settle(() => owner.get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "typed");
  await first.dispose();
});

test("a write the server rejects does not block the corrected text", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "base");
  const owner = first.document(id);
  await settle(() => owner.get().phase === "saved");
  await owner.change({ type: "write", body: "OVERSIZE", attachments: [] });
  await settle(() => owner.get().phase === "error");
  assertEquals(owner.get().error, "Too large");
  assertEquals(owner.get().document?.body, "OVERSIZE");
  // Polling does not upload the same rejected content again.
  const sent = f.requests.filter((p) => p === "/api/drafts/mutations").length;
  await first.refresh();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(
    f.requests.filter((p) => p === "/api/drafts/mutations").length,
    sent,
  );
  assertEquals(owner.get().phase, "error");
  await owner.change({ type: "write", body: "fits now", attachments: [] });
  await settle(() => owner.get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "fits now");
  assertEquals(f.server.get(id)?.body_revision, 2);
  await first.dispose();
});

test("writes queued offline fold behind the one already dispatched", async () => {
  const f = fixture();
  const first = f.create();
  await first.start();
  const id = await first.create("Draft", null, "document", "base");
  const owner = first.document(id);
  await settle(() => owner.get().phase === "saved");
  f.offline(true);
  for (const body of ["one", "one two", "one two three"]) {
    await owner.change({ type: "write", body, attachments: [] });
  }
  // The first one was dispatched and keeps its identity; the rest fold.
  await settle(() => f.local.get(id)?.pending.length === 2);
  assertEquals(owner.get().document?.body, "one two three");
  // A title change after the fold still reaches the sidebar index.
  await owner.change({ type: "rename", title: "Renamed" });
  assertEquals(first.get().entries.find((e) => e.id === id)?.title, "Renamed");
  f.offline(false);
  const sent = f.requests.filter((p) => p === "/api/drafts/mutations").length;
  await owner.retry();
  await settle(() => owner.get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "one two three");
  assertEquals(f.server.get(id)?.body_revision, 3);
  assertEquals(f.server.get(id)?.title, "Renamed");
  assertEquals(
    f.requests.filter((p) => p === "/api/drafts/mutations").length,
    sent + 3,
  );
  await first.dispose();
});

test("a write whose reply was lost is retried, not merged with its own text", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "a");
  const owner = first.document(id);
  await settle(() => owner.get().phase === "saved");
  f.dropReply();
  await owner.change({ type: "write", body: "a b", attachments: [] });
  await settle(() => owner.get().phase === "local");
  assertEquals(f.server.get(id)?.body, "a b");
  // Issue both writes before either can be dispatched: whether a write that
  // has already left folds with a later one depends on the runtime's task
  // ordering, which is not what this test is about.
  await Promise.all(["a b c", "a b c d"].map((body) =>
    owner.change({ type: "write", body, attachments: [] })
  ));
  await settle(() => owner.get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "a b c d");
  assertEquals(f.server.get(id)?.body_revision, 3);
  await first.dispose();
});

test("two devices editing through outages converge without losing a word", async () => {
  for (let seed = 1; seed <= 60; seed++) {
    let state = seed;
    const random = (bound: number): number => {
      state = (state * 1103515245 + 12345) & 0x7fffffff;
      return state % bound;
    };
    const f = fixture();
    const devices = [f.device(), f.device()];
    const id = await devices[0]!.repository.create(
      "Shared",
      null,
      "document",
      "alpha beta gamma delta",
    );
    await settle(() =>
      devices[0]!.repository.document(id).get().phase === "saved"
    );
    await devices[1]!.repository.document(id).refresh();
    const typed: string[] = [];
    for (let step = 0; step < 30; step++) {
      const device = devices[random(2)]!;
      const owner = device.repository.document(id);
      const action = random(10);
      if (action < 6) {
        const body = owner.get().document!.body;
        const words = body.split(" ");
        const word = `w${typed.length}`;
        typed.push(word);
        words.splice(random(words.length + 1), 0, word);
        await owner.change(
          { type: "write", body: words.join(" "), attachments: [] },
          undefined,
          { body, attachments: [] },
        );
      } else if (action < 8) device.offline(random(2) === 0);
      else if (action === 8) f.dropReply();
      else await owner.refresh();
      if (random(3) === 0) await new Promise((r) => setTimeout(r, 1));
    }
    for (const device of devices) device.offline(false);
    const owners = devices.map((d) => d.repository.document(id));
    for (let round = 0; round < 50; round++) {
      for (const owner of owners) {
        await owner.retry();
        await owner.refresh();
      }
      await new Promise((r) => setTimeout(r, 2));
      const body = f.server.get(id)!.body;
      if (
        owners.every((o) =>
          o.get().phase === "saved" && o.get().document?.body === body
        )
      ) break;
    }
    const body = f.server.get(id)!.body;
    for (const owner of owners) {
      assertEquals(owner.get().phase, "saved", `seed ${seed}`);
      assertEquals(owner.get().document?.body, body, `seed ${seed}`);
    }
    const words = body.split(" ");
    for (const word of ["alpha", "beta", "gamma", "delta", ...typed]) {
      assertEquals(
        words.filter((w) => w === word).length,
        1,
        `seed ${seed}: ${word} in ${body}`,
      );
    }
    assertEquals(f.server.size, 1, `seed ${seed}`);
    assertEquals(f.notices, [], `seed ${seed}`);
    for (const device of devices) await device.repository.dispose();
  }
});

test("a rejected write does not hold back a rename, and stays reported", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Draft", null, "document", "base");
  const owner = first.document(id);
  await settle(() => owner.get().phase === "saved");
  await owner.change({ type: "write", body: "OVERSIZE", attachments: [] });
  await settle(() => owner.get().phase === "error");
  await owner.change({ type: "rename", title: "Renamed" });
  await settle(() => f.server.get(id)?.title === "Renamed");
  await settle(() => owner.get().phase === "error");
  assertEquals(owner.get().error, "Too large");
  assertEquals(owner.get().document?.body, "OVERSIZE");
  assertEquals(f.server.get(id)?.body, "base");
  // Polling leaves the rejected text alone.
  const sent = f.requests.filter((p) => p === "/api/drafts/mutations").length;
  await first.refresh();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assertEquals(
    f.requests.filter((p) => p === "/api/drafts/mutations").length,
    sent,
  );
  await owner.change({ type: "write", body: "fits now", attachments: [] });
  await settle(() => owner.get().phase === "saved");
  assertEquals(f.server.get(id)?.body, "fits now");
  assertEquals(f.server.get(id)?.title, "Renamed");
  await first.dispose();
});

test("a rename queued offline outranks a newer cached index entry", async () => {
  const f = fixture();
  const first = f.create();
  const id = await first.create("Old", null, "document", "body");
  await settle(() => first.document(id).get().phase === "saved");
  await first.dispose();
  // The index later heard of a newer revision this replica never fetched.
  f.cacheIndex([{ ...f.server.get(id)!, title: "Elsewhere", revision: 5 }]);
  f.offline(true);
  const second = f.create();
  await second.start();
  assertEquals(
    second.get().entries.find((e) => e.id === id)?.title,
    "Elsewhere",
  );
  await second.document(id).change({ type: "rename", title: "Mine" });
  assertEquals(second.get().entries.find((e) => e.id === id)?.title, "Mine");
  await second.dispose();
});
