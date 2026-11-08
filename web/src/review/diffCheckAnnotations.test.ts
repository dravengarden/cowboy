import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { diffCheckAnnotations } from "./diffCheckAnnotations";
import { diffPointToNewFile } from "./diffSourceModel";
import type { CodeDiagnostic } from "./codeApi";

const finding: CodeDiagnostic = {
  start: { row: 10, column: 2 },
  end: { row: 10, column: 100 },
  severity: 1,
  message: "type mismatch",
};

test("check findings map to added and context lines, never removed lines", () => {
  const text = "@@ -11,2 +11,2 @@\n-old value\n+new value\n context";
  const point = (row: number, column: number) => diffPointToNewFile(text, row, column);
  const result = diffCheckAnnotations(text, [finding], point);
  assertEquals(result, [
    {
      ...finding,
      start: { row: 2, column: 3 },
      end: { row: 2, column: 10 },
    },
  ]);
  assertEquals(
    diffCheckAnnotations(
      text,
      [
        {
          ...finding,
          start: { row: 11, column: 0 },
        },
      ],
      point,
    )[0]?.start,
    { row: 3, column: 1 },
  );
});

test("unmatched or historical projections and absent lines have no marks", () => {
  assertEquals(
    diffCheckAnnotations("+new", [finding], () => null),
    [],
  );
  assertEquals(
    diffCheckAnnotations("+new", [finding], () => ({ row: 0, column: 0 })),
    [],
  );
});

test("findings cannot mark a neighbouring source line", () => {
  assertEquals(
    diffCheckAnnotations("+x", [finding], () => ({ row: 9, column: 0 })),
    [],
  );
});

test("line-end findings mark the last character, including empty added lines", () => {
  assertEquals(
    diffCheckAnnotations("+x", [finding], () => ({ row: 10, column: 0 }))[0]?.start.column,
    1,
  );
  assertEquals(
    diffCheckAnnotations("+", [finding], () => ({ row: 10, column: 0 }))[0]?.start.column,
    0,
  );
});
