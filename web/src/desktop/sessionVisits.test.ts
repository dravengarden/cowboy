import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { DraftMetadata } from "../documents/model.ts";
import type { SessionMeta } from "../protocol.ts";
import {
  DESKTOP_VISIT_LIMIT,
  desktopRecentItems,
  withDesktopVisit,
} from "./sessionVisits.ts";

const session = (id: string, title: string): SessionMeta =>
  ({ id, title, provider: "codex", cwd: "/tmp", status: "running" }) as
    SessionMeta;
const draft = (id: string, title: string, deleted = false): DraftMetadata => ({
  id,
  kind: "document",
  title,
  parent_id: null,
  revision: 1,
  body_revision: 1,
  metadata_revision: 1,
  updated_at_ms: 0,
  deleted,
});

test("a visit moves to the front without duplicates", () => {
  let visits = withDesktopVisit([], "a", 1);
  visits = withDesktopVisit(visits, "b", 2);
  visits = withDesktopVisit(visits, "a", 3);
  assertEquals(visits.map((visit) => visit.key), ["a", "b"]);
  for (let index = 0; index < DESKTOP_VISIT_LIMIT + 5; index++) {
    visits = withDesktopVisit(visits, `s${String(index)}`, index);
  }
  assertEquals(visits.length, DESKTOP_VISIT_LIMIT);
});

test("Recent skips the current item and anything deleted", () => {
  const visits = [
    { key: "a", at: 5 },
    { key: "draft:d1", at: 4 },
    { key: "gone", at: 3 },
    { key: "draft:d2", at: 2 },
    { key: "b", at: 1 },
  ];
  const items = desktopRecentItems(
    visits,
    "a",
    [session("a", "Alpha"), session("b", "Beta")],
    [draft("d1", "Notes"), draft("d2", "Old", true)],
  );
  assertEquals(items.map((item) => [item.key, item.kind, item.title]), [
    ["draft:d1", "draft", "Notes"],
    ["b", "session", "Beta"],
  ]);
  assertEquals(
    desktopRecentItems(visits, null, [session("a", "A"), session("b", "B")], [], 1)
      .map((item) => item.key),
    ["a"],
  );
});
