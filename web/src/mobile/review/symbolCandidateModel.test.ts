import { assertEquals } from "jsr:@std/assert";
import { rankInspectCandidates } from "./symbolCandidateModel.ts";

Deno.test("symbol ranking prefers a nearby identifier over a tapped keyword", () => {
  const ranked = rankInspectCandidates([
    { label: "struct", containsTap: true, distance: 0, column: 4 },
    {
      label: "RiskDecision",
      containsTap: false,
      distance: 8,
      column: 11,
    },
  ]);
  assertEquals(ranked.map((candidate) => candidate.label), [
    "RiskDecision",
    "struct",
  ]);
});

Deno.test("symbol ranking keeps the tapped identifier ahead of its neighbors", () => {
  const ranked = rankInspectCandidates([
    { label: "left", containsTap: false, distance: 3, column: 0 },
    { label: "target", containsTap: true, distance: 0, column: 5 },
    { label: "right", containsTap: false, distance: 2, column: 12 },
  ]);
  assertEquals(ranked.map((candidate) => candidate.label), [
    "target",
    "right",
    "left",
  ]);
});
