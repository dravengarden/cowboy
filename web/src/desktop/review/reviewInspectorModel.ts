// What the Inspector shows beside the Document (docs/desktop-code-review.md
// "Inspector"), as rows of the Document: a hunk header, a symbol, a problem.
// Rows are one-based Document lines; in a diff a file line is placed only
// through a verified projection, never guessed from the patch text.

import type { CodeCheckDiagnostic, CodeChecks, CodeDocumentSymbol } from "../../mobile/review/codeApi";
import { checkTitle } from "../../mobile/review/checkAnnotations";
import { diffHunkLines } from "../../mobile/review/diffNavigationModel";

export interface InspectorHunk {
  readonly row: number;
  readonly header: string;
  readonly added: number;
  readonly removed: number;
}

export function inspectorHunks(text: string): InspectorHunk[] {
  const lines = text.split("\n");
  return diffHunkLines(text).map((row, index, all) => {
    const end = (all[index + 1] ?? lines.length + 1) - 1;
    let added = 0;
    let removed = 0;
    for (let at = row; at < end; at++) {
      const line = lines[at] ?? "";
      if (line.startsWith("+") && !line.startsWith("+++")) added++;
      else if (line.startsWith("-") && !line.startsWith("---")) removed++;
    }
    return { row, header: lines[row - 1] ?? "", added, removed };
  });
}

export interface InspectorProblem {
  readonly key: string;
  readonly severity: CodeCheckDiagnostic["severity"];
  readonly message: string;
  readonly source: string;
  /** The file line the check reported (one-based). */
  readonly line: number;
  /** Where it is in the Document, or undefined when it cannot be placed. */
  readonly row: number | undefined;
}

const ORDER = { error: 0, warning: 1, note: 2 } as const;

function samePath(a: string, b: string): boolean {
  const normal = (path: string) => path.replace(/^(\.\/)+/, "");
  return normal(a) === normal(b);
}

/**
 * The latest check findings for `path`. `rowOf` places a file line in the
 * Document: identity for a source file, the verified projection for a diff,
 * nothing when neither applies. Rows come in Document order, then file order.
 */
export function inspectorProblems(
  checks: CodeChecks | undefined,
  path: string,
  rowOf: (line: number) => number | undefined,
): InspectorProblem[] {
  if (!checks) return [];
  const problems = checks.runs.flatMap((run) =>
    run.diagnostics
      .filter((diagnostic) => diagnostic.line > 0 && samePath(diagnostic.path, path))
      .map((diagnostic, index): InspectorProblem => ({
        key: `${run.runId}:${String(index)}`,
        severity: diagnostic.severity,
        message: `${diagnostic.code ? `[${diagnostic.code}] ` : ""}${diagnostic.message}`,
        source: checkTitle(checks, run.check),
        line: diagnostic.line,
        row: rowOf(diagnostic.line),
      }))
  );
  return problems.sort((left, right) =>
    (left.row ?? Number.MAX_SAFE_INTEGER) - (right.row ?? Number.MAX_SAFE_INTEGER) ||
    left.line - right.line || ORDER[left.severity] - ORDER[right.severity]
  );
}

export interface InspectorSymbol {
  readonly name: string;
  readonly kind: number;
  readonly depth: number;
  readonly line: number;
  readonly row: number | undefined;
}

export function inspectorOutline(
  symbols: readonly CodeDocumentSymbol[],
  rowOf: (line: number) => number | undefined,
  depth = 0,
): InspectorSymbol[] {
  return symbols.flatMap((symbol) => {
    const line = symbol.selectionStart.row + 1;
    return [
      { name: symbol.name, kind: symbol.kind, depth, line, row: rowOf(line) },
      ...inspectorOutline(symbol.children, rowOf, depth + 1),
    ];
  });
}
