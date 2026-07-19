// A compact line-level diff for the edit/write tool cards. ACP hands us the
// `oldText` + `newText` of a changed region (see the `{type:"diff"}` content
// block); we turn that into a classic unified-diff string (` `/`-`/`+` line
// prefixes) so the existing Markdown ```diff fence can syntax-highlight it the
// same way it highlights any other code — no extra dependency, no diff widget.
//
// Most providers send a small changed region, but some send the entire file.
// Strip common edges before LCS and bound the remaining matrix: a one-line edit
// in an 11k-line file must not allocate/visit ~121 million cells on the UI
// thread. The display keeps a compact amount of surrounding context while the
// original old/new text remains available to Raw/copy paths.

const CONTEXT_LINES = 4;
const MAX_LCS_CELLS = 250_000;

/** Longest-common-subsequence table → the aligned ` `/`-`/`+` line list. */
function lcsLines(a: string[], b: string[]): { sign: " " | "-" | "+"; text: string }[] {
  const n = a.length;
  const m = b.length;
  // dp[i][j] = LCS length of a[i:] and b[j:]. One extra row/col of zeros.
  const dp: number[][] = Array.from({ length: n + 1 }, () => Array.from<number>({ length: m + 1 }).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i]![j] = a[i] === b[j] ? (dp[i + 1]![j + 1]! + 1) : Math.max(dp[i + 1]![j]!, dp[i]![j + 1]!);
    }
  }
  const out: { sign: " " | "-" | "+"; text: string }[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ sign: " ", text: a[i]! });
      i++;
      j++;
    } else if (dp[i + 1]![j]! >= dp[i]![j + 1]!) {
      out.push({ sign: "-", text: a[i]! });
      i++;
    } else {
      out.push({ sign: "+", text: b[j]! });
      j++;
    }
  }
  while (i < n) out.push({ sign: "-", text: a[i++]! });
  while (j < m) out.push({ sign: "+", text: b[j++]! });
  return out;
}

export interface DiffResult {
  /** A unified-diff string (one `' '|'-'|'+'` prefix per line) for ```diff. */
  text: string;
  added: number;
  removed: number;
}

/** Build a unified-diff string + add/remove counts from old → new text. */
export function unifiedDiff(oldText: string, newText: string): DiffResult {
  // Trailing newline split would add a phantom empty last line on both sides;
  // strip ONE trailing newline first so the diff doesn't show a spurious change.
  const a = oldText.replace(/\n$/, "").split("\n");
  const b = newText.replace(/\n$/, "").split("\n");
  let prefix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix++;
  let suffix = 0;
  while (
    suffix < a.length - prefix &&
    suffix < b.length - prefix &&
    a[a.length - 1 - suffix] === b[b.length - 1 - suffix]
  ) suffix++;

  const oldMiddle = a.slice(prefix, a.length - suffix);
  const newMiddle = b.slice(prefix, b.length - suffix);
  const middle = oldMiddle.length * newMiddle.length <= MAX_LCS_CELLS
    ? lcsLines(oldMiddle, newMiddle)
    : [
      ...oldMiddle.map((text) => ({ sign: "-" as const, text })),
      ...newMiddle.map((text) => ({ sign: "+" as const, text })),
    ];
  const lines: { sign: " " | "-" | "+"; text: string }[] = [];
  const prefixStart = Math.max(0, prefix - CONTEXT_LINES);
  if (prefixStart > 0) lines.push({ sign: " ", text: `… ${prefixStart} unchanged lines …` });
  for (let index = prefixStart; index < prefix; index++) {
    lines.push({ sign: " ", text: a[index]! });
  }
  lines.push(...middle);
  const suffixShown = Math.min(CONTEXT_LINES, suffix);
  for (let index = 0; index < suffixShown; index++) {
    lines.push({ sign: " ", text: a[a.length - suffix + index]! });
  }
  if (suffix > suffixShown) {
    lines.push({ sign: " ", text: `… ${suffix - suffixShown} unchanged lines …` });
  }
  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l.sign === "+") added++;
    else if (l.sign === "-") removed++;
  }
  return { text: lines.map((l) => l.sign + l.text).join("\n"), added, removed };
}
