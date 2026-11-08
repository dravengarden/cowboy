import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import type { CodeChecks } from "../../review/codeApi.ts";
import { inspectorHunks, inspectorOutline, inspectorProblems, withLanguageProblems } from "./reviewInspectorModel.ts";

test("hunks name their header row and count their lines", () => {
  const text = "diff --git a/x b/x\n@@ -1,2 +1,2 @@\n-a\n+b\n+c\n x\n@@ -9 +9 @@\n-d\n";
  assertEquals(inspectorHunks(text), [
    { row: 2, header: "@@ -1,2 +1,2 @@", added: 2, removed: 1 },
    { row: 7, header: "@@ -9 +9 @@", added: 0, removed: 1 },
  ]);
});

const checks: CodeChecks = {
  apiVersion: 1,
  allowed: true,
  declared: [{ id: "lint", title: "Lint", format: "lines", timeoutSeconds: 30 }],
  runs: [{
    runId: "r1", check: "lint", state: "failed", startedAtMs: 0, outputTruncated: false, truncated: false,
    diagnostics: [
      { path: "./src/a.ts", line: 9, column: 1, severity: "warning", message: "late" },
      { path: "src/a.ts", line: 3, column: 2, severity: "error", code: "E1", message: "early" },
      { path: "src/b.ts", line: 1, column: 1, severity: "error", message: "other file" },
      { path: "src/a.ts", line: 0, column: 0, severity: "note", message: "no line" },
    ],
  }],
};

test("problems of this file come in Document order, unplaced last", () => {
  const placed = inspectorProblems(checks, "src/a.ts", (line) => line);
  assertEquals(placed.map((p) => [p.row, p.message, p.source]), [[3, "[E1] early", "Lint"], [9, "late", "Lint"]]);
  const unplaced = inspectorProblems(checks, "src/a.ts", (line) => line === 9 ? 4 : undefined);
  assertEquals(unplaced.map((p) => p.row), [4, undefined]);
  assertEquals(inspectorProblems(undefined, "src/a.ts", (line) => line), []);
});

test("the outline flattens with depth and places each symbol", () => {
  const point = (row: number) => ({ row, column: 0 });
  const symbols = [{
    name: "A", kind: 5, start: point(0), end: point(9), selectionStart: point(0), selectionEnd: point(0),
    children: [{ name: "b", kind: 6, start: point(2), end: point(3), selectionStart: point(2), selectionEnd: point(2), children: [] }],
  }];
  assertEquals(inspectorOutline(symbols, (line) => line + 100).map((s) => [s.name, s.depth, s.line, s.row]), [
    ["A", 0, 1, 101],
    ["b", 1, 3, 103],
  ]);
});

test("language diagnostics join the check findings in Document order", () => {
  const point = (row: number) => ({ row, column: 0 });
  const merged = withLanguageProblems(inspectorProblems(checks, "src/a.ts", (line) => line), [
    { start: point(4), end: point(4), severity: 1, source: "rust-analyzer", message: "mismatched types" },
    { start: point(0), end: point(0), severity: 4, message: "hint" },
  ]);
  assertEquals(merged.map((p) => [p.row, p.severity, p.source]), [
    [1, "note", "Language"], [3, "error", "Lint"], [5, "error", "rust-analyzer"], [9, "warning", "Lint"],
  ]);
});
