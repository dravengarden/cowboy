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

Deno.test("competing writer stays recoverable instead of silently adopting a new base revision", async () => {
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
  await first.document(id).change({
    type: "write",
    body: "my unsaved typing",
    attachments: [],
  }, 1);
  await settle(() => first.document(id).get().phase === "conflict");
  assertEquals(first.document(id).get().document?.body, "my unsaved typing");
  assertEquals(first.document(id).get().remote?.body, "other device");
  assertEquals(f.server.get(id)?.body, "other device");
  const copy = await first.create(
    "Recovered",
    null,
    "document",
    first.document(id).get().document!.body,
  );
  await settle(() => first.document(copy).get().phase === "saved");
  await first.document(id).useRemote();
  assertEquals(first.document(id).get().document?.body, "other device");
  assertEquals(f.server.get(copy)?.body, "my unsaved typing");
  await first.dispose();
});
