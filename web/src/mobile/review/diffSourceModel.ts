export interface DiffSourcePoint {
  row: number;
  column: number;
}

/**
 * Build an offset-preserving source projection for a unified diff.
 *
 * Replacing the one-character diff marker with a space keeps every source
 * token at its display offset while hiding headers from the language parser.
 */
export function diffSourceProjection(text: string): string {
  return text.split("\n").map((line) => {
    if (
      (line.startsWith("+") && !line.startsWith("+++")) ||
      (line.startsWith("-") && !line.startsWith("---")) ||
      line.startsWith(" ")
    ) {
      return ` ${line.slice(1)}`;
    }
    return " ".repeat(line.length);
  }).join("\n");
}

/** Map a CodeMirror diff coordinate to the current working-tree buffer. */
export function diffPointToNewFile(
  text: string,
  displayRow: number,
  displayColumn: number,
): DiffSourcePoint | null {
  if (displayRow < 0 || displayColumn < 1) return null;
  const lines = text.split("\n");
  let newRow = 0;
  let insideHunk = false;

  for (let row = 0; row < lines.length; row += 1) {
    const line = lines[row] ?? "";
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/u.exec(line);
    if (hunk) {
      newRow = Number(hunk[1]) - 1;
      insideHunk = true;
      continue;
    }
    if (!insideHunk) continue;
    if (row === displayRow) {
      if (
        line.startsWith("-") ||
        line.startsWith("\\") ||
        line.startsWith("@@")
      ) return null;
      return {
        row: newRow,
        column: Math.max(0, displayColumn - 1),
      };
    }
    if (!line.startsWith("-") && !line.startsWith("\\")) newRow += 1;
  }
  return null;
}
