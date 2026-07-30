import { assertEquals } from "jsr:@std/assert";
import {
  rankAndDedupeInspectCandidates,
  rankInspectCandidates,
} from "./symbolCandidateModel.ts";

function candidate(
  label: string,
  overrides: Partial<{
    containsTap: boolean;
    horizontalDistance: number;
    verticalDistance: number;
    rowDistance: number;
    row: number;
    column: number;
  }> = {},
) {
  return {
    label,
    containsTap: false,
    horizontalDistance: 0,
    verticalDistance: 0,
    rowDistance: 0,
    row: 0,
    column: 0,
    ...overrides,
  };
}

Deno.test("symbol ranking prefers a nearby identifier over a tapped keyword", () => {
  const ranked = rankInspectCandidates([
    candidate("struct", { containsTap: true, column: 4 }),
    candidate("RiskDecision", { horizontalDistance: 8, column: 11 }),
  ]);
  assertEquals(ranked.map((candidate) => candidate.label), [
    "RiskDecision",
    "struct",
  ]);
});

Deno.test("symbol ranking keeps the tapped identifier ahead of its neighbors", () => {
  const ranked = rankInspectCandidates([
    candidate("left", { horizontalDistance: 3 }),
    candidate("target", { containsTap: true, column: 5 }),
    candidate("right", { horizontalDistance: 2, column: 12 }),
  ]);
  assertEquals(ranked.map((candidate) => candidate.label), [
    "target",
    "right",
    "left",
  ]);
});

Deno.test("symbol ranking uses weighted two-dimensional screen distance", () => {
  const ranked = rankInspectCandidates([
    candidate("nextLine", {
      verticalDistance: 12,
      rowDistance: 1,
      row: 1,
    }),
    candidate("sameLine", { horizontalDistance: 14, column: 20 }),
  ]);
  assertEquals(ranked.map((item) => item.label), [
    "sameLine",
    "nextLine",
  ]);
});

Deno.test("symbol candidates deduplicate labels after choosing the nearest occurrence", () => {
  const ranked = rankAndDedupeInspectCandidates([
    candidate("Usage", {
      horizontalDistance: 32,
      row: 3,
      rowDistance: 3,
    }),
    candidate("Usage", {
      horizontalDistance: 4,
      row: 0,
      column: 8,
    }),
    candidate("Serialize", { horizontalDistance: 12, column: 20 }),
  ], 12);
  assertEquals(ranked.map((item) => [item.label, item.row]), [
    ["Usage", 0],
    ["Serialize", 0],
  ]);
});

Deno.test("symbol candidate limit applies after deduplication", () => {
  const ranked = rankAndDedupeInspectCandidates([
    candidate("a", { horizontalDistance: 1 }),
    candidate("a", { horizontalDistance: 2, column: 1 }),
    candidate("b", { horizontalDistance: 3, column: 2 }),
    candidate("c", { horizontalDistance: 4, column: 3 }),
  ], 2);
  assertEquals(ranked.map((item) => item.label), ["a", "b"]);
});
