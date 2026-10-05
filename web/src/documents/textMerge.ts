/** Three-way text merge for independently edited copies of one document.
 *
 * Like Obsidian Sync's Markdown merge, non-overlapping edits from both sides
 * are combined and an overlapping region keeps both versions (remote first)
 * instead of discarding either. Tokens are words, whitespace runs, single
 * punctuation marks and single CJK characters, so concurrent edits in one
 * paragraph merge without duplicating the whole line. */

export interface TextChange {
  readonly from: number;
  readonly to: number;
  readonly insert: string;
}

interface Hunk {
  /** Replaced base token range. */
  readonly start: number;
  readonly end: number;
  readonly insert: readonly string[];
}

const TOKEN =
  /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]|[^\s\p{P}\p{S}\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]+|\s+|[^]/gu;
/** Edit budget per tokenization; it bounds the trace to a few megabytes. */
const MAX_EDITS = 1000;

function words(text: string): string[] {
  return text.match(TOKEN) ?? [];
}

function lines(text: string): string[] {
  return text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
}

/** Myers' O(ND) diff over interned tokens. Returns null past `MAX_EDITS`. */
function diff(a: readonly string[], b: readonly string[]): Hunk[] | null {
  let prefix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) {
    prefix++;
  }
  let suffix = 0;
  while (
    suffix < a.length - prefix && suffix < b.length - prefix &&
    a[a.length - 1 - suffix] === b[b.length - 1 - suffix]
  ) suffix++;
  const ids = new Map<string, number>();
  const intern = (token: string): number => {
    let id = ids.get(token);
    if (id === undefined) ids.set(token, id = ids.size);
    return id;
  };
  const x = a.slice(prefix, a.length - suffix).map(intern);
  const y = b.slice(prefix, b.length - suffix).map(intern);
  const n = x.length;
  const m = y.length;
  if (n === 0 && m === 0) return [];
  if (n === 0 || m === 0) {
    return [{
      start: prefix,
      end: prefix + n,
      insert: b.slice(prefix, prefix + m),
    }];
  }
  const max = Math.min(n + m, MAX_EDITS);
  const offset = max + 1;
  let v = new Int32Array(2 * max + 3);
  const trace: Int32Array[] = [];
  let found = false;
  for (let d = 0; d <= max && !found; d++) {
    trace.push(v);
    const next = v.slice();
    for (let k = -d; k <= d; k += 2) {
      let i = k === -d || (k !== d && v[offset + k - 1]! < v[offset + k + 1]!)
        ? v[offset + k + 1]!
        : v[offset + k - 1]! + 1;
      let j = i - k;
      while (i < n && j < m && x[i] === y[j]) {
        i++;
        j++;
      }
      next[offset + k] = i;
      if (i >= n && j >= m) {
        found = true;
        break;
      }
    }
    v = next;
  }
  if (!found) return null;
  // Walk the trace backwards into matched token pairs.
  const matches: Array<[number, number]> = [];
  let i = n;
  let j = m;
  for (let d = trace.length - 1; d >= 0; d--) {
    const previous = trace[d]!;
    const k = i - j;
    const down = k === -d ||
      (k !== d && previous[offset + k - 1]! < previous[offset + k + 1]!);
    const prevK = down ? k + 1 : k - 1;
    const prevI = d === 0 ? 0 : previous[offset + prevK]!;
    const prevJ = prevI - prevK;
    while (i > prevI && j > prevJ && i > 0 && j > 0) {
      i--;
      j--;
      matches.push([i, j]);
    }
    if (d > 0) {
      i = prevI;
      j = prevJ;
    }
  }
  matches.reverse();
  const hunks: Hunk[] = [];
  let ai = 0;
  let bj = 0;
  for (const [mi, mj] of [...matches, [n, m] as [number, number]]) {
    if (mi > ai || mj > bj) {
      hunks.push({
        start: prefix + ai,
        end: prefix + mi,
        insert: b.slice(prefix + bj, prefix + mj),
      });
    }
    ai = mi + 1;
    bj = mj + 1;
  }
  return hunks;
}

function overlaps(start: number, end: number, hunk: Hunk): boolean {
  if (start === end && hunk.start === hunk.end) return start === hunk.start;
  if (hunk.start === hunk.end) return start < hunk.start && hunk.start < end;
  if (start === end) return hunk.start < start && start < hunk.end;
  return hunk.start < end && start < hunk.end;
}

function apply(
  base: readonly string[],
  start: number,
  end: number,
  hunks: readonly Hunk[],
): string {
  let out = "";
  let at = start;
  for (const hunk of hunks) {
    out += base.slice(at, hunk.start).join("") + hunk.insert.join("");
    at = hunk.end;
  }
  return out + base.slice(at, end).join("");
}

function mergeTokens(
  base: readonly string[],
  ours: readonly string[],
  theirs: readonly string[],
): string | null {
  const mine = diff(base, ours);
  const remote = diff(base, theirs);
  if (!mine || !remote) return null;
  const tagged = [
    ...remote.map((hunk) => ({ hunk, ours: false })),
    ...mine.map((hunk) => ({ hunk, ours: true })),
  ].sort((l, r) =>
    l.hunk.start - r.hunk.start ||
    (l.hunk.end - l.hunk.start) - (r.hunk.end - r.hunk.start) ||
    Number(l.ours) - Number(r.ours)
  );
  let out = "";
  let at = 0;
  for (let index = 0; index < tagged.length;) {
    const group = [tagged[index]!];
    let start = group[0]!.hunk.start;
    let end = group[0]!.hunk.end;
    while (
      index + group.length < tagged.length &&
      overlaps(start, end, tagged[index + group.length]!.hunk)
    ) {
      const next = tagged[index + group.length]!.hunk;
      group.push(tagged[index + group.length]!);
      start = Math.min(start, next.start);
      end = Math.max(end, next.end);
    }
    index += group.length;
    out += base.slice(at, start).join("");
    at = end;
    const theirsPart = apply(
      base,
      start,
      end,
      group.filter((g) => !g.ours).map((g) => g.hunk),
    );
    const oursPart = apply(
      base,
      start,
      end,
      group.filter((g) => g.ours).map((g) => g.hunk),
    );
    const original = base.slice(start, end).join("");
    if (theirsPart === oursPart || oursPart === original) out += theirsPart;
    else if (theirsPart === original) out += oursPart;
    else out += theirsPart + oursPart;
  }
  return out + base.slice(at).join("");
}

/** Merge `ours` and `theirs`, both edited from `base`. Null only when the
 * texts are too divergent to align within a bounded cost. */
export function mergeText(
  base: string,
  ours: string,
  theirs: string,
): string | null {
  if (ours === theirs || ours === base) return theirs;
  if (theirs === base) return ours;
  return mergeTokens(words(base), words(ours), words(theirs)) ??
    mergeTokens(lines(base), lines(ours), lines(theirs));
}

/** Minimal edits turning `before` into `after`, in ascending `before` offsets,
 * so an editor can map its selection through a remote update. */
export function textChanges(before: string, after: string): TextChange[] {
  if (before === after) return [];
  const a = words(before);
  const hunks = diff(a, words(after));
  if (!hunks) return [{ from: 0, to: before.length, insert: after }];
  const offsets = [0];
  for (const token of a) offsets.push(offsets.at(-1)! + token.length);
  return hunks.map((hunk) => ({
    from: offsets[hunk.start]!,
    to: offsets[hunk.end]!,
    insert: hunk.insert.join(""),
  }));
}

/** Map one offset through `changes` (assoc: stay after an insertion at it). */
export function mapOffset(
  offset: number,
  changes: readonly TextChange[],
): number {
  let delta = 0;
  for (const change of changes) {
    if (
      change.from >= offset &&
      !(change.from === change.to && change.from === offset)
    ) break;
    if (change.to <= offset) {
      delta += change.insert.length - (change.to - change.from);
    } else {
      // The offset was inside replaced text: land at the end of the insert.
      return change.from + delta + change.insert.length;
    }
  }
  return offset + delta;
}
