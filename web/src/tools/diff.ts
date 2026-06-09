// A compact line-level diff for the edit/write tool cards. ACP hands us the
// `oldText` + `newText` of a changed region (see the `{type:"diff"}` content
// block); we turn that into a classic unified-diff string (` `/`-`/`+` line
// prefixes) so the existing Markdown ```diff fence can syntax-highlight it the
// same way it highlights any other code — no extra dependency, no diff widget.
//
// The algorithm is a textbook LCS over lines (Hunt–McIlroy), which is plenty for
// the small regions an edit tool reports. We don't do hunk headers / context
// trimming — the region is already the "hunk", and showing it whole reads better
// in a chat card than `@@` math.

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
  const lines = lcsLines(a, b);
  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l.sign === "+") added++;
    else if (l.sign === "-") removed++;
  }
  return { text: lines.map((l) => l.sign + l.text).join("\n"), added, removed };
}
