import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  effectiveUndoState,
  parseUndoEntries,
  pruneUndoEntries,
  reviveUndoEntry,
  serializeUndoEntries,
  trashUndoBlocker,
  UNDO_MAX_ENTRIES,
  UNDO_NOTICE_MS,
  UNDO_TTL_MS,
  type UndoEntry,
} from "./undoHistory";

function entry(id: string, createdAt: number, extra: Partial<UndoEntry> = {}): UndoEntry {
  return {
    id,
    message: id,
    createdAt,
    principal: "u",
    steps: [{ kind: "restore_draft", id }],
    state: "available",
    ...extra,
  };
}

test("one notice duration and a day of history", () => {
  assertEquals(UNDO_NOTICE_MS, 3_000);
  assertEquals(UNDO_TTL_MS, 86_400_000);
});

test("pruning drops expired entries, sorts newest first and caps the count", () => {
  const now = UNDO_TTL_MS * 2;
  const kept = pruneUndoEntries([
    entry("old", now - UNDO_TTL_MS),
    entry("a", now - 10),
    entry("b", now - 5),
  ], now);
  assertEquals(kept.map((e) => e.id), ["b", "a"]);
  const many = Array.from({ length: UNDO_MAX_ENTRIES + 5 }, (_, i) => entry(`e${i}`, now - i));
  assertEquals(pruneUndoEntries(many, now).length, UNDO_MAX_ENTRIES);
});

test("an undo interrupted by a reload is never offered again", () => {
  const revived = reviveUndoEntry(entry("x", 1, { state: "undoing" }));
  assertEquals(revived.state, "unavailable");
  assertEquals(reviveUndoEntry(entry("y", 1)).state, "available");
});

test("oversized steps stay in memory and read as unavailable elsewhere", () => {
  const big = entry("big", 1, {
    steps: [{
      kind: "copy_to_session",
      sessionId: "s",
      text: "x".repeat(300 * 1024),
      attachments: [],
    }],
  });
  const stored = parseUndoEntries(serializeUndoEntries([big, entry("small", 2)]));
  assertEquals(stored.map((e) => e.steps === null), [true, false]);
  assertEquals(effectiveUndoState(stored[0]!).state, "unavailable");
  assertEquals(effectiveUndoState(stored[1]!).state, "available");
});

test("malformed storage is ignored instead of breaking the history", () => {
  assertEquals(parseUndoEntries("not json"), []);
  assertEquals(parseUndoEntries(JSON.stringify([{ id: 1 }, entry("ok", 1)])).length, 1);
  assertEquals(
    parseUndoEntries(JSON.stringify([entry("bad", 1, { steps: [{ kind: "rm -rf" }] as never })])),
    [],
  );
});

test("a Trash restore reports why it can no longer run", () => {
  const view = {
    ready: true,
    liveSessions: new Set(["restored"]),
    trashedSessions: new Set(["trashed"]),
    folders: new Map([["f-trashed", true], ["f-live", false]]),
  };
  const steps = (sessions: string[], folders: string[] = []) => [{ kind: "restore_trash" as const, sessions, folders }];
  assertEquals(trashUndoBlocker(steps(["trashed"], ["f-trashed"]), view), undefined);
  assertEquals(trashUndoBlocker(steps(["restored"]), view), "Already restored from Trash.");
  assertEquals(trashUndoBlocker(steps(["gone"]), view), "No longer in Trash: deleted permanently.");
  assertEquals(trashUndoBlocker(steps([], ["f-live"]), view), "Already restored from Trash.");
  assertEquals(trashUndoBlocker(steps([], ["f-gone"]), view), "No longer in Trash: deleted permanently.");
  // A cold boot has no Controller view yet: never guess.
  assertEquals(trashUndoBlocker(steps(["gone"]), { ...view, ready: false }), undefined);
});
