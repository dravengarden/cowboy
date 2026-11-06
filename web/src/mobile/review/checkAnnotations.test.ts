import { test } from "bun:test";
import { assert, assertEquals } from "../../../../tools/test-assert.ts";
import {
  checkDiagnosticsForPath,
  checkStatusSummary,
  withCheckDiagnostics,
} from "./checkAnnotations";
import type { CodeCheckRun, CodeChecks, CodeLanguage } from "./codeApi";

const NOW = 1_800_000_000_000;

function run(overrides: Partial<CodeCheckRun>): CodeCheckRun {
  return {
    runId: "op-1",
    check: "clippy",
    state: "failed",
    startedAtMs: NOW - 300_000,
    finishedAtMs: NOW - 240_000,
    exitCode: 101,
    outputTruncated: false,
    diagnostics: [],
    truncated: false,
    ...overrides,
  };
}

function checks(runs: CodeCheckRun[]): CodeChecks {
  return {
    apiVersion: 1,
    allowed: true,
    declared: [{
      id: "clippy",
      title: "Clippy",
      format: "cargo-json",
      timeoutSeconds: 600,
    }],
    runs,
  };
}

const findings = checks([run({
  diagnostics: [
    {
      path: "src/main.rs",
      line: 3,
      column: 5,
      severity: "error",
      code: "E0425",
      message: "cannot find value",
    },
    {
      path: "./src/main.rs",
      line: 9,
      column: 0,
      severity: "warning",
      message: "unused",
    },
    { path: "src/lib.rs", line: 1, column: 1, severity: "note", message: "x" },
  ],
})]);

test("check findings for a file become 0-based viewer diagnostics", () => {
  const diagnostics = checkDiagnosticsForPath(findings, "src/main.rs");
  assertEquals(diagnostics.length, 2);
  assertEquals(diagnostics[0]?.start, { row: 2, column: 4 });
  assertEquals(diagnostics[0]?.severity, 1);
  assertEquals(diagnostics[0]?.message, "Clippy: [E0425] cannot find value");
  // No column: the rest of the line is marked.
  assertEquals(diagnostics[1]?.start, { row: 8, column: 0 });
  assertEquals(diagnostics[1]?.severity, 2);
  assert((diagnostics[1]?.end.column ?? 0) > 1000);
  assertEquals(checkDiagnosticsForPath(undefined, "src/main.rs"), []);
});

test("check diagnostics join the language engine's, or stand alone", () => {
  const diagnostics = checkDiagnosticsForPath(findings, "src/main.rs");
  const synthesized = withCheckDiagnostics(
    undefined,
    "src/main.rs",
    diagnostics,
  );
  assertEquals(synthesized?.diagnostics.length, 2);
  assertEquals(synthesized?.semanticTokens, []);
  const language: CodeLanguage = {
    apiVersion: 1,
    path: "src/main.rs",
    version: [],
    diagnostics: [{
      start: { row: 0, column: 0 },
      end: { row: 0, column: 1 },
      severity: 1,
      message: "lsp",
    }],
    inlayHints: [],
    semanticTokens: [1, 2, 3, 0, 0],
  };
  const merged = withCheckDiagnostics(language, "src/main.rs", diagnostics);
  assertEquals(merged?.diagnostics.length, 3);
  assertEquals(merged?.semanticTokens, language.semanticTokens);
  assertEquals(withCheckDiagnostics(language, "src/main.rs", []), language);
  assertEquals(withCheckDiagnostics(undefined, "a", []), undefined);
});

test("the status line counts this file's findings and dates each run", () => {
  assertEquals(checkStatusSummary(undefined, "src/main.rs", NOW), undefined);
  assertEquals(checkStatusSummary(checks([]), "src/main.rs", NOW), undefined);
  const status = checkStatusSummary(findings, "src/main.rs", NOW);
  assertEquals(status?.severity, "error");
  assertEquals(status?.summary, "Checks · 1 error, 1 warning in this file");
  assert(status?.detail.includes("Clippy: failed 4 min ago"));
  assert(
    status?.detail.includes("3 findings, 1 error, 1 warning in this file"),
  );
  const elsewhere = checkStatusSummary(findings, "README.md", NOW);
  assertEquals(elsewhere?.severity, "success");
  assertEquals(elsewhere?.summary, "Checks · no findings in this file");
  const running = checkStatusSummary(
    checks([run({ state: "running", finishedAtMs: undefined })]),
    "src/main.rs",
    NOW,
  );
  assertEquals(running?.severity, "info");
  assertEquals(running?.summary, "Checks running · Clippy");
});
