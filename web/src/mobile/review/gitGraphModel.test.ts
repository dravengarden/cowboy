import { assertEquals } from "jsr:@std/assert";
import type { GitCommitSummary } from "./codeApi.ts";
import { buildGitGraph } from "./gitGraphModel.ts";

const commit = (oid: string, parents: string[]): GitCommitSummary => ({
  oid,
  parents,
  author: "A",
  authoredAt: "2026-08-02T00:00:00Z",
  subject: oid,
  decorations: [],
});

Deno.test("git graph keeps first-parent line and exposes merge edge", () => {
  const graph = buildGitGraph([
    commit("merge", ["main", "topic"]),
    commit("topic", ["base"]),
    commit("main", ["base"]),
    commit("base", []),
  ]);
  assertEquals(graph[0].nodeLane, 0);
  assertEquals(
    graph[0].edges.filter((edge) => edge.kind === "parent").map((edge) => edge.to),
    [0, 1],
  );
  assertEquals(graph[1].nodeLane, 1);
  assertEquals(graph[2].nodeLane, 0);
  assertEquals(graph[3].nodeLane, 0);
});
