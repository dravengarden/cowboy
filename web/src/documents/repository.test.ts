import { assertEquals, assertRejects } from "jsr:@std/assert";
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
  let index: unknown = null;
  const create = () =>
    createDraftRepository({
      signal: lifetime.signal,
      persistence: (
        id,
      ): LocalPersistence<ClientSnapshot<DraftDocument | null>> => ({
        load: () => Promise.resolve(local.get(id) ?? null),
        save: (snapshot) => {
          if (blocked) return Promise.reject(new Error("quota"));
          local.set(id, structuredClone(snapshot));
          return Promise.resolve();
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
        if (offline) return Promise.reject(new Error("offline"));
        requests.push(path);
        if (path === "/api/drafts") {
          return Promise.resolve(
            Response.json({ entries: [...server.values()] }),
          );
        }
        if (path === "/api/drafts/mutations") {
          const args = JSON.parse(String(init?.body)) as DraftMutationArgs & {
            operation_id: string;
          };
          const current = server.get(args.document_id) ?? null;
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
  return {
    create,
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
  };
}
async function settle(check: () => boolean): Promise<void> {
  for (let i = 0; i < 100; i++) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
  throw new Error("Draft owner did not settle");
}

Deno.test("storage failure never sends authored content", async () => {
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

Deno.test("offline create and edits survive reload and replay exactly once", async () => {
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

Deno.test("an ancestor-less competing write is kept as a copy and never blocks the document", async () => {
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

Deno.test("an ancestor-less write merges through the server's recovery history", async () => {
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

Deno.test("a write refused by a newer server text merges both edits and resends", async () => {
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

Deno.test("an editor writing against stale content merges with the newer replica", async () => {
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

Deno.test("a refused rename is reapplied over the newer metadata", async () => {
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

Deno.test("a pushed revision refreshes only an open document that lacks it", async () => {
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
