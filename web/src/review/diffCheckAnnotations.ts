import type { CodeDiagnostic } from "./codeApi";
import type { DiffSourcePoint } from "./diffSourceModel";

/** Project findings only through a verified working-tree diff mapping. Deleted
 * lines, headers and historical patches have no working-tree coordinates. */
export function diffCheckAnnotations(
  text: string,
  diagnostics: CodeDiagnostic[],
  point: (row: number, column: number) => DiffSourcePoint | null,
): CodeDiagnostic[] {
  const byRow = new Map<number, CodeDiagnostic[]>();
  for (const diagnostic of diagnostics) {
    const row = diagnostic.start.row;
    const entries = byRow.get(row) ?? [];
    entries.push(diagnostic);
    byRow.set(row, entries);
  }
  return text.split("\n").flatMap((line, row) => {
    const source = point(row, 1);
    if (!source) return [];
    return (byRow.get(source.row) ?? []).flatMap((diagnostic) => {
      const column = Math.min(diagnostic.start.column + 1, Math.max(0, line.length - 1));
      return [
        {
          ...diagnostic,
          start: { row, column },
          end: { row, column: line.length },
        },
      ];
    });
  });
}
