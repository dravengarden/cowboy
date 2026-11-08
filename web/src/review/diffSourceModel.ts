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

/** Gutter labels for a unified diff, one per display row. */
export interface DiffLineLabels {
  /** Working-tree line for context/added rows; empty for headers and removals. */
  readonly labels: readonly string[];
  /** The longest label, used to size the gutter before rows render. */
  readonly widest: string;
}

/**
 * Number each display row with its line in the new file, as the selection
 * reference sent to an Agent does, instead of its row in the patch text.
 */
export function diffNewLineLabels(text: string): DiffLineLabels {
  let newRow: number | undefined;
  let widest = "";
  const labels = text.split("\n").map((line) => {
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/u.exec(line);
    if (hunk) {
      newRow = Number(hunk[1]);
      return "";
    }
    if (line.startsWith("diff ")) newRow = undefined;
    if (
      newRow === undefined ||
      !(line.startsWith("+") || line.startsWith(" "))
    ) return "";
    const label = String(newRow);
    newRow += 1;
    if (label.length > widest.length) widest = label;
    return label;
  });
  return { labels, widest };
}
