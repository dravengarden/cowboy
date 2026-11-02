/** One-based, inclusive lines of the document on screen. */
export interface ReviewLineSelection {
  readonly from: number;
  readonly to: number;
}

/**
 * A tap starts a selection, a second tap extends it, and a tap on the only
 * selected line, or on a range already drawn, clears or restarts it.
 */
export function nextReviewLineSelection(
  current: ReviewLineSelection | undefined,
  line: number,
): ReviewLineSelection | undefined {
  if (!current) return { from: line, to: line };
  if (current.from !== current.to) return { from: line, to: line };
  if (current.from === line) return undefined;
  return {
    from: Math.min(current.from, line),
    to: Math.max(current.from, line),
  };
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/** File lines covered by a selection in a unified diff, by side. */
export function diffSelectionLines(
  text: string,
  selection: ReviewLineSelection,
): { readonly old: number[]; readonly new: number[] } {
  const covered = { old: [] as number[], new: [] as number[] };
  let oldLine: number | undefined;
  let newLine: number | undefined;
  text.split("\n").forEach((line, index) => {
    const number = index + 1;
    const selected = number >= selection.from && number <= selection.to;
    const hunk = HUNK.exec(line);
    if (hunk) {
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      return;
    }
    if (oldLine === undefined || newLine === undefined) return;
    if (line.startsWith("\\")) return;
    if (line.startsWith("+")) {
      if (selected) covered.new.push(newLine);
      newLine += 1;
    } else if (line.startsWith("-")) {
      if (selected) covered.old.push(oldLine);
      oldLine += 1;
    } else {
      if (selected) covered.new.push(newLine);
      oldLine += 1;
      newLine += 1;
    }
  });
  return covered;
}

function lineRange(lines: readonly number[]): string {
  const first = lines[0];
  const last = lines.at(-1);
  return first === last ? `${first}` : `${first}-${last}`;
}

/** A reference an Agent can open: new-side lines, or old-side when only those. */
function selectionLocation(
  path: string,
  text: string,
  kind: "source" | "diff",
  selection: ReviewLineSelection,
): string {
  if (kind === "source") {
    return `${path}:${lineRange([selection.from, selection.to])}`;
  }
  const covered = diffSelectionLines(text, selection);
  if (covered.new.length > 0) return `${path}:${lineRange(covered.new)}`;
  if (covered.old.length > 0) return `${path} (removed ${lineRange(covered.old)})`;
  return path;
}

function fence(text: string): string {
  const longest = Math.max(
    2,
    ...[...text.matchAll(/^`{3,}/gm)].map((match) => match[0].length),
  );
  return "`".repeat(longest + 1);
}

/** The Composer text for a selection; the question is the reader's to add. */
export function reviewSelectionMessage(
  { path, text, kind, selection, comparison }: {
    readonly path: string;
    readonly text: string;
    readonly kind: "source" | "diff";
    readonly selection: ReviewLineSelection;
    readonly comparison?: string | undefined;
  },
): string {
  const snippet = text.split("\n")
    .slice(selection.from - 1, selection.to)
    .join("\n");
  const marker = fence(snippet);
  const language = kind === "diff" ? "diff" : "";
  const against = comparison
    ? ` (compared with ${comparison.replace(/^refs\/cowboy\/session-bases\/.*/, "session start")
      .replace(/^(committed:)?refs\/(heads|remotes)\//, "")})`
    : "";
  return `\`${selectionLocation(path, text, kind, selection)}\`${against}\n` +
    `${marker}${language}\n${snippet}\n${marker}\n\n`;
}

/**
 * The first message of a fork asked to explain a selection. A fork already
 * knows why the code changed, so it answers from that before re-reading, and
 * its worktree is its own copy, so changing nothing is a request for focus.
 */
export function reviewExplainPrompt(
  selection: string,
  viewedRoot?: string | undefined,
): string {
  const where = viewedRoot
    ? `\nThe reader is looking at ${viewedRoot}, not this worktree.`
    : "";
  return "Explain this code to the person reviewing it: what it does and why " +
    "it is written this way, using what you remember of this work first. " +
    `Do not change any files.${where}\n\n${selection}`;
}

const EXPLAIN_TITLE = "Explain · ";

export function reviewExplainTitle(path: string): string {
  return `${EXPLAIN_TITLE}${path.split("/").at(-1)}`;
}

/** This session's Explain forks, oldest first. The list is the record. */
export function reviewExplainSessions<
  T extends { id: string; title: string; forked_from?: string | undefined },
>(sessions: readonly T[], source: string): T[] {
  return sessions
    .filter((session) =>
      session.forked_from === source && session.title.startsWith(EXPLAIN_TITLE)
    )
    .toSorted((left, right) => left.id.localeCompare(right.id, undefined, { numeric: true }));
}
