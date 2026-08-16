import { assertEquals } from "jsr:@std/assert";
import type { GitCommitSummary } from "./codeApi.ts";
import {
  historyPageCursor,
  mergeHistoryPage,
} from "./reviewHistoryPaging.ts";

const commit = (oid: string): GitCommitSummary => ({
  oid,
  parents: [],
  author: "A",
  authoredAt: "2026-08-16T00:00:00Z",
  subject: oid,
  decorations: [],
});

Deno.test("history pages append unseen commits and stop on a repeated page", () => {
  const first = mergeHistoryPage([], [commit("a"), commit("b")], true);
  assertEquals(first.commits.map((row) => row.oid), ["a", "b"]);
  assertEquals(first.truncated, true);
  assertEquals(historyPageCursor(first.commits), "b");

  const next = mergeHistoryPage(first.commits, [commit("c")], true);
  assertEquals(next.commits.map((row) => row.oid), ["a", "b", "c"]);
  assertEquals(next.truncated, true);

  const repeated = mergeHistoryPage(next.commits, [commit("a"), commit("b")], true);
  assertEquals(repeated.commits.map((row) => row.oid), ["a", "b", "c"]);
  assertEquals(repeated.truncated, false);
});
