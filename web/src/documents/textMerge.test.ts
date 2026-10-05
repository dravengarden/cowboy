import { assertEquals } from "jsr:@std/assert";
import { mapOffset, mergeText, textChanges } from "./textMerge.ts";

function applyChanges(
  text: string,
  changes: ReturnType<typeof textChanges>,
): string {
  let out = "";
  let at = 0;
  for (const change of changes) {
    out += text.slice(at, change.from) + change.insert;
    at = change.to;
  }
  return out + text.slice(at);
}

Deno.test("edits in different paragraphs both survive", () => {
  const base = "Intro line\n\nMiddle\n\nOutro line\n";
  assertEquals(
    mergeText(
      base,
      "Intro line edited here\n\nMiddle\n\nOutro line\n",
      "Intro line\n\nMiddle\n\nOutro line from phone\n",
    ),
    "Intro line edited here\n\nMiddle\n\nOutro line from phone\n",
  );
});

Deno.test("edits to different words of one line merge without duplication", () => {
  assertEquals(
    mergeText(
      "the quick brown fox jumps",
      "the slow brown fox jumps",
      "the quick brown fox leaps",
    ),
    "the slow brown fox leaps",
  );
});

Deno.test("CJK text merges per character", () => {
  assertEquals(
    mergeText("今天天气很好", "今天天气非常好", "明天天气很好"),
    "明天天气非常好",
  );
});

Deno.test("one-sided changes adopt that side exactly", () => {
  assertEquals(mergeText("a", "a", "b"), "b");
  assertEquals(mergeText("a", "b", "a"), "b");
  assertEquals(mergeText("a", "b", "b"), "b");
});

Deno.test("overlapping edits keep both versions, remote first", () => {
  assertEquals(
    mergeText("Title: draft\n", "Title: mine\n", "Title: theirs\n"),
    "Title: theirsmine\n",
  );
  assertEquals(mergeText("", "local", "remote"), "remotelocal");
});

Deno.test("appends on both devices keep both", () => {
  assertEquals(
    mergeText("List\n- a\n", "List\n- a\n- mine\n", "List\n- a\n- phone\n"),
    "List\n- a\n- phone\n- mine\n",
  );
});

Deno.test("a deletion and an unrelated edit both apply", () => {
  assertEquals(
    mergeText(
      "keep one\ndrop this\nkeep two\n",
      "keep one\nkeep two\n",
      "keep one\ndrop this\nkeep two!\n",
    ),
    "keep one\nkeep two!\n",
  );
});

Deno.test("text changes reproduce the target and map a caret", () => {
  const before = "alpha beta gamma delta";
  const after = "ALPHA beta gamma delta!";
  const changes = textChanges(before, after);
  assertEquals(applyChanges(before, changes), after);
  // Caret after "gamma" stays after "gamma".
  const caret = before.indexOf(" delta");
  assertEquals(after.slice(0, mapOffset(caret, changes)), "ALPHA beta gamma");
  assertEquals(mapOffset(before.length, changes), after.length);
});

Deno.test("large unrelated rewrites still merge through the line fallback", () => {
  const base = Array.from({ length: 400 }, (_, i) => `line ${i}`).join("\n");
  const ours = base.replace("line 3\n", "line three\n");
  const theirs = Array.from(
    { length: 400 },
    (_, i) => i === 3 ? "line 3" : `rewritten ${i} words more words`,
  ).join("\n");
  const merged = mergeText(base, ours, theirs);
  assertEquals(merged?.includes("line three"), true);
  assertEquals(merged?.includes("rewritten 399 words more words"), true);
});

Deno.test("randomized disjoint edits always merge and changes round-trip", () => {
  let seed = 7;
  const random = (n: number): number => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed % n;
  };
  const vocabulary = ["a", "b", "草稿", "同步", " ", "\n", ".", "word"];
  const text = (length: number): string =>
    Array.from({ length }, () => vocabulary[random(vocabulary.length)]).join(
      "",
    );
  for (let round = 0; round < 300; round++) {
    const head = text(random(40));
    const middle = text(1 + random(10));
    const tail = text(random(40));
    const base = head + "|" + middle + "|" + tail;
    const ours = text(random(40)) + "|" + middle + "|" + tail;
    const theirs = head + "|" + middle + "|" + text(random(40));
    const merged = mergeText(base, ours, theirs)!;
    assertEquals(merged.startsWith(ours.slice(0, ours.indexOf("|"))), true);
    assertEquals(merged.endsWith(theirs.slice(theirs.lastIndexOf("|") + 1)), true);
    assertEquals(applyChanges(base, textChanges(base, ours)), ours);
    assertEquals(applyChanges(theirs, textChanges(theirs, merged)), merged);
  }
});
