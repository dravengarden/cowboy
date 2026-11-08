// Full-file context for a single-file unified diff (docs/desktop-code-review.md
// "Expanding context is not reconstructed from the worktree"). The old side is
// read by the blob id the diff's own `index` line names, so it is the exact
// text the patch was computed from. Outside the hunks both sides are equal,
// so the gaps are filled from the old side alone. Every context and removed
// line of the patch must match the blob at its position; any mismatch, a
// second file, a binary or truncated diff refuses the expansion.

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;
const NO_NEWLINE = "\\ No newline at end of file";

/** The old side's blob id, or undefined when there is none to read (a new
 *  file, a binary or mode-only change, or more than one file). */
export function diffBaseOid(text: string): string | undefined {
  let files = 0;
  let oid: string | undefined;
  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) files++;
    else if (line.startsWith("@@")) break;
    const index = /^index ([0-9a-f]{7,64})\.\.[0-9a-f]{7,64}/.exec(line);
    if (index) oid = index[1];
  }
  if (files > 1 || !oid || /^0+$/.test(oid)) return undefined;
  return oid;
}

interface Hunk {
  oldFirst: number;
  oldCount: number;
  newFirst: number;
  newCount: number;
  label: string;
  lines: string[];
}

/** First line a hunk side covers: `-a,0` sits after line `a`. */
function first(start: number, count: number): number {
  return count > 0 ? start : start + 1;
}

function parse(text: string): { header: string[]; hunks: Hunk[] } | undefined {
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  const header: string[] = [];
  const hunks: Hunk[] = [];
  for (const line of lines) {
    const match = HUNK.exec(line);
    if (match) {
      const oldCount = match[2] === undefined ? 1 : Number(match[2]);
      const newCount = match[4] === undefined ? 1 : Number(match[4]);
      hunks.push({
        oldFirst: first(Number(match[1]), oldCount),
        oldCount,
        newFirst: first(Number(match[3]), newCount),
        newCount,
        label: match[5] ?? "",
        lines: [],
      });
    } else if (hunks.length === 0) {
      header.push(line);
    } else {
      hunks.at(-1)!.lines.push(line);
    }
  }
  return hunks.length > 0 ? { header, hunks } : undefined;
}

/** Whether the hunk's old-side lines are the blob's lines at its position. */
function matches(hunk: Hunk, old: readonly string[]): boolean {
  let line = hunk.oldFirst;
  let seen = 0;
  for (const row of hunk.lines) {
    if (row === NO_NEWLINE || row.startsWith("+")) continue;
    if (!row.startsWith(" ") && !row.startsWith("-")) return false;
    if (old[line - 1] !== row.slice(1)) return false;
    line++;
    seen++;
  }
  return seen === hunk.oldCount;
}

function header(hunk: Hunk): string {
  return `@@ -${String(hunk.oldFirst)},${String(hunk.oldCount)} +${String(hunk.newFirst)},${String(hunk.newCount)} @@${hunk.label}`;
}

/**
 * `diff` with every unchanged line of the file as context, or undefined when
 * the patch and the blob do not agree. Each gap joins the hunk before it
 * (the leading one the first hunk), so hunk headers stay just above their
 * changes and `]c` lands where it did.
 */
export function expandDiffContext(diff: string, base: string): string | undefined {
  const parsed = parse(diff);
  if (!parsed) return undefined;
  const missingNewline = base.length > 0 && !base.endsWith("\n");
  const old = base.split("\n");
  if (!missingNewline) old.pop();
  const { hunks } = parsed;
  if (!hunks.every((hunk) => matches(hunk, old))) return undefined;
  for (let index = 1; index < hunks.length; index++) {
    if (hunks[index]!.oldFirst < hunks[index - 1]!.oldFirst + hunks[index - 1]!.oldCount) return undefined;
  }
  const context = (from: number, to: number): string[] =>
    old.slice(from - 1, to - 1).map((line) => ` ${line}`);
  const leading = hunks[0]!.oldFirst - 1;
  if (leading > 0) {
    const hunk = hunks[0]!;
    hunk.lines = [...context(1, hunk.oldFirst), ...hunk.lines];
    hunk.oldFirst = 1;
    hunk.oldCount += leading;
    hunk.newFirst -= leading;
    hunk.newCount += leading;
    if (hunk.newFirst < 1) return undefined;
  }
  for (let index = 0; index < hunks.length; index++) {
    const hunk = hunks[index]!;
    const from = hunk.oldFirst + hunk.oldCount;
    const to = index + 1 < hunks.length ? hunks[index + 1]!.oldFirst : old.length + 1;
    if (to <= from) continue;
    // Old and new both end inside this hunk: nothing follows it.
    if (hunk.lines.includes(NO_NEWLINE)) return undefined;
    const gap = context(from, to);
    hunk.lines.push(...gap);
    hunk.oldCount += gap.length;
    hunk.newCount += gap.length;
    if (to === old.length + 1 && missingNewline) hunk.lines.push(NO_NEWLINE);
  }
  return [...parsed.header, ...hunks.flatMap((hunk) => [header(hunk), ...hunk.lines])].join("\n") + "\n";
}
