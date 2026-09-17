/** A local coordinate proof, never a native grant or a Git/causal snapshot. */
import type { DiffSourcePoint } from "./diffSourceModel.ts";

declare const projected: unique symbol;
export interface ReviewDiffProjection {
  readonly [projected]: true;
  readonly patch: string;
  /** Complete LF working-file text, not the patch or an ETag. */
  readonly source: string;
}
const rows = new WeakMap<ReviewDiffProjection, ReadonlyMap<number, string>>();
const targets = new WeakMap<
  ReviewDiffProjection,
  ReadonlyMap<number, number>
>();
const MAX_LINES = 100_000;

function lines(text: string, bytes: number): string[] | undefined {
  if (
    text.length > bytes || new TextEncoder().encode(text).length > bytes ||
    text.includes("\r") || /[\uD800-\uDFFF]/u.test(text)
  ) return undefined;
  let count = 1, offset = 0;
  while ((offset = text.indexOf("\n", offset)) !== -1) {
    if (++count > MAX_LINES) return undefined;
    offset++;
  }
  return text.split("\n");
}

/** Validate EVERY new-side line, hunk count, range and EOF marker before
 * exposing any coordinate. Hidden working-file lines are captured too, but
 * this deliberately makes no claim about the diff's old side or Git revision.
 */
export function projectReviewDiff(
  patch: string,
  source: string,
): ReviewDiffProjection | undefined {
  const diff = lines(patch, 8 * 1024 * 1024);
  const file = lines(source, 4 * 1024 * 1024);
  if (!diff || !file) return undefined;
  if (source.endsWith("\n") || source === "") file.pop();
  const mapping = new Map<number, number>();
  const displayed = new Map<number, string>();
  let oldRow = 0, newRow = 0, oldLeft = 0, newLeft = 0;
  let oldEnd = 0, newEnd = 0, hunks = 0, files = 0;
  let previous: { newRow?: number; marked: boolean } | undefined;
  const previousComplete = () =>
    previous?.newRow === undefined || previous.marked ||
    previous.newRow < file.length - 1 || source.endsWith("\n");
  for (let row = 0; row < diff.length; row++) {
    const line = diff[row]!;
    if (line === "\\ No newline at end of file") {
      if (!previous || previous.marked) return undefined;
      if (
        previous.newRow !== undefined &&
        (previous.newRow !== file.length - 1 || source.endsWith("\n"))
      ) return undefined;
      previous.marked = true;
      continue;
    }
    if (!previousComplete()) return undefined;
    previous = undefined;
    if (line.startsWith("@@")) {
      if (oldLeft || newLeft) return undefined;
      const header = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?: .*)?$/u
        .exec(line);
      if (!header) return undefined;
      const oldStart = Number(header[1]), newStart = Number(header[3]);
      oldLeft = Number(header[2] ?? 1);
      newLeft = Number(header[4] ?? 1);
      if (
        ![oldStart, newStart, oldLeft, newLeft].every((n) =>
          Number.isSafeInteger(n) && n >= 0 && n <= 0xffffffff
        ) || (oldLeft > 0 && oldStart === 0) ||
        (newLeft > 0 && newStart === 0)
      ) return undefined;
      oldRow = oldStart - (oldLeft > 0 ? 1 : 0);
      newRow = newStart - (newLeft > 0 ? 1 : 0);
      if (
        oldRow < oldEnd || newRow < newEnd ||
        newRow + newLeft > file.length || (!oldLeft && !newLeft)
      ) return undefined;
      oldEnd = oldRow + oldLeft;
      newEnd = newRow + newLeft;
      hunks++;
      continue;
    }
    if (hunks && (oldLeft || newLeft)) {
      const marker = line[0];
      if (marker !== " " && marker !== "+" && marker !== "-") {
        return undefined;
      }
      if (marker !== "+" && --oldLeft < 0) return undefined;
      if (marker !== "-") {
        if (--newLeft < 0 || file[newRow] !== line.slice(1)) return undefined;
        displayed.set(row, line.slice(1));
        mapping.set(row, newRow);
        previous = { newRow, marked: false };
        newRow++;
      } else previous = { marked: false };
      continue;
    }
    // Only a final newline is allowed after a complete hunk. A second file,
    // stray body line or unsupported combined/conflict patch fails closed.
    if (hunks) {
      if (line !== "" || row !== diff.length - 1) return undefined;
    } else if (line.startsWith("diff --git ")) {
      if (++files > 1) return undefined;
    } else if (
      !/^(?:index |--- |\+\+\+ |new file mode |deleted file mode |old mode |new mode |similarity index |dissimilarity index |rename from |rename to |copy from |copy to )/u
        .test(line)
    ) return undefined;
  }
  if (oldLeft || newLeft || !previousComplete() || !mapping.size) {
    return undefined;
  }
  const projection = Object.freeze({ patch, source }) as ReviewDiffProjection;
  rows.set(projection, displayed);
  targets.set(projection, mapping);
  return projection;
}

/** Zero-based display UTF-16 -> current-file UTF-16; never clamp a bad point. */
export function projectedDiffPoint(
  projection: ReviewDiffProjection,
  row: number,
  column: number,
): DiffSourcePoint | null {
  const text = rows.get(projection)?.get(row);
  const target = targets.get(projection)?.get(row);
  if (
    !Number.isSafeInteger(row) || row < 0 || !Number.isSafeInteger(column) ||
    column < 1 || text === undefined || target === undefined ||
    column - 1 > text.length
  ) return null;
  const at = column - 1;
  const before = text.charCodeAt(at - 1), after = text.charCodeAt(at);
  if (
    before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff
  ) return null;
  return { row: target, column: at };
}
