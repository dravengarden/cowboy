import { unifiedDiff } from "./diff";

Deno.test("unified diff keeps exact small edits", () => {
  const result = unifiedDiff("one\ntwo\nthree", "one\nchanged\nthree");
  if (result.added !== 1 || result.removed !== 1) throw new Error("incorrect edit counts");
  if (!result.text.includes("-two") || !result.text.includes("+changed")) {
    throw new Error("changed lines are missing");
  }
});

Deno.test("whole-file edit trims unchanged edges before diffing", () => {
  const oldLines = Array.from({ length: 11_000 }, (_, index) => `line ${index}`);
  const newLines = [...oldLines];
  newLines[5_500] = "line changed";
  const result = unifiedDiff(oldLines.join("\n"), newLines.join("\n"));
  if (result.added !== 1 || result.removed !== 1) throw new Error("incorrect large edit counts");
  if (!result.text.includes("-line 5500") || !result.text.includes("+line changed")) {
    throw new Error("large edit lost its changed line");
  }
  if (result.text.split("\n").length > 12 || !result.text.includes("unchanged lines")) {
    throw new Error("large unchanged edges should be compacted into context markers");
  }
});
