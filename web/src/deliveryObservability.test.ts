import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "@std/assert";

test("local delivery failures are reported before callers can swallow them", async () => {
  const source = await readFile(
    new URL("./store.ts", import.meta.url), "utf8",
  );
  const start = source.indexOf('"delivery_persist_started"');
  const persist = source.indexOf("await mutateQueueDurably", start);
  const failed = source.indexOf('"delivery_persist_failed"', persist);
  const rethrow = source.indexOf("throw error;", failed);
  const completed = source.indexOf('"delivery_persist_completed"', rethrow);
  assert(start >= 0 && persist > start && failed > persist);
  assert(rethrow > failed && completed > rethrow);
  const reporting = source.slice(failed, rethrow);
  assert(reporting.includes("notify("));
  assert(reporting.includes("session_id: sessionId"));
  assert(reporting.includes("mutation_id: cmid"));
  assert(!reporting.includes("row.text"));
  assert(!reporting.includes("attachments"));
  const rollback = source.slice(persist, failed);
  assert(
    rollback.includes(
      "reconcileOptimistic(state.optimisticMessages, sessionId, new Set([cmid]))",
    ),
  );
  assert(!rollback.includes("clearDraft"));
});
