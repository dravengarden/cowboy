import { assertEquals } from "jsr:@std/assert";
import {
  documentRefreshDecision,
  lineAnchorTarget,
  normalizedAnchorText,
  textAnchorIndex,
} from "./documentRefreshModel.ts";

Deno.test("a worktree change to another file never refreshes the open document", () => {
  for (const reading of [true, false]) {
    assertEquals(
      documentRefreshDecision({
        currentRevision: "r1",
        nextRevision: "r1",
        currentText: "old",
        nextText: "old",
        reading,
        now: 10_000,
        autoApplyUntil: 0,
      }),
      "ignore",
    );
  }
});

Deno.test("a changed document prompts only while it is being read", () => {
  const changed = {
    currentRevision: "r1",
    nextRevision: "r2",
    currentText: "old",
    nextText: "new",
    now: 10_000,
    autoApplyUntil: 0,
  };
  assertEquals(
    documentRefreshDecision({ ...changed, reading: true }),
    "prompt",
  );
  assertEquals(
    documentRefreshDecision({ ...changed, reading: false }),
    "apply",
  );
  // Returning to Review revalidates immediately; that change was never seen.
  assertEquals(
    documentRefreshDecision({
      ...changed,
      reading: true,
      autoApplyUntil: 12_500,
    }),
    "apply",
  );
});

Deno.test("a result without a revision is compared by its text", () => {
  const base = {
    currentRevision: undefined,
    nextRevision: undefined,
    reading: true,
    now: 10_000,
    autoApplyUntil: 0,
  };
  assertEquals(
    documentRefreshDecision({ ...base, currentText: "a", nextText: "a" }),
    "ignore",
  );
  assertEquals(
    documentRefreshDecision({ ...base, currentText: "a", nextText: "b" }),
    "prompt",
  );
});

Deno.test("text anchors keep their ordinal among repeated blocks", () => {
  const texts = ["Intro", "Note", "Body", "Note", "Tail"];
  assertEquals(textAnchorIndex(texts, { text: "Note", occurrence: 1 }), 3);
  // Content inserted above the anchor shifts its index, not its identity.
  assertEquals(
    textAnchorIndex(["New", ...texts], { text: "Body", occurrence: 0 }),
    3,
  );
  // One duplicate was removed: fall back to the last remaining match.
  assertEquals(
    textAnchorIndex(["Note", "Body"], { text: "Note", occurrence: 1 }),
    0,
  );
  assertEquals(
    textAnchorIndex(texts, { text: "Gone", occurrence: 0 }),
    undefined,
  );
  assertEquals(normalizedAnchorText("  a\n  b\t c "), "a b c");
});

Deno.test("line anchors follow moved content and clamp otherwise", () => {
  const lines = ["a", "b", "inserted", "inserted", "target", "c"];
  const text = (line: number): string => lines[line - 1]!;
  assertEquals(
    lineAnchorTarget(lines.length, text, { line: 3, text: "target" }),
    5,
  );
  assertEquals(
    lineAnchorTarget(lines.length, text, { line: 40, text: "missing" }),
    6,
  );
  // Blank lines are ambiguous; keep the numeric position.
  assertEquals(lineAnchorTarget(lines.length, text, { line: 2, text: "" }), 2);
});
