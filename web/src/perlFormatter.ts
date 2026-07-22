const QUOTE_LIKE_OPERATORS = ["tr", "qq", "qw", "qx", "qr", "s", "y", "m", "q"] as const;

const PAIRED_DELIMITERS: Readonly<Record<string, string>> = {
  "(": ")",
  "[": "]",
  "{": "}",
  "<": ">",
};

function scanDelimited(source: string, opening: number): number {
  const open = source[opening];
  if (!open) return opening;
  const close = PAIRED_DELIMITERS[open] ?? open;
  const paired = close !== open;
  let depth = 1;
  let escaped = false;
  for (let index = opening + 1; index < source.length; index += 1) {
    const char = source[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\") {
      escaped = true;
      continue;
    }
    if (paired && char === open) depth += 1;
    if (char !== close) continue;
    depth -= 1;
    if (depth === 0) return index;
  }
  return source.length - 1;
}

function quoteLikeEnd(source: string, start: number): number | null {
  const previous = source[start - 1];
  if (previous && /[\p{L}\p{N}_$]/u.test(previous)) return null;
  const operator = QUOTE_LIKE_OPERATORS.find((candidate) => source.startsWith(candidate, start));
  if (!operator) return null;
  let delimiter = start + operator.length;
  const next = source[delimiter];
  if (next && /[\p{L}\p{N}_]/u.test(next)) return null;
  while (/\s/u.test(source[delimiter] ?? "")) delimiter += 1;
  const open = source[delimiter];
  if (!open || /[\p{L}\p{N}_\s]/u.test(open)) return null;

  let end = scanDelimited(source, delimiter);
  if (operator !== "s" && operator !== "tr" && operator !== "y") return end;

  if (PAIRED_DELIMITERS[open]) {
    let replacement = end + 1;
    while (/\s/u.test(source[replacement] ?? "")) replacement += 1;
    if (!source[replacement]) return end;
    return scanDelimited(source, replacement);
  }

  // With an unpaired delimiter (`s#old#new#g`) the first closing delimiter is
  // also the separator; scan the replacement until the next unescaped one.
  let escaped = false;
  for (let index = end + 1; index < source.length; index += 1) {
    const char = source[index];
    if (escaped) {
      escaped = false;
    } else if (char === "\\") {
      escaped = true;
    } else if (char === open) {
      return index;
    }
  }
  return source.length - 1;
}

/**
 * Reflow a display-only Perl payload at statement boundaries. This is not a
 * source rewriter: the Shell Source/copy paths retain the exact ACP bytes.
 * Quote-like operators are skipped as atomic ranges so delimiters and
 * semicolons inside regex/replacement bodies never become layout boundaries.
 */
export function reflowPerl(source: string, _columns: number): string {
  const trimmed = source.trim();
  let result = "";
  let quote: "'" | '"' | "`" | null = null;
  let escaped = false;
  let lineStart = true;

  for (let index = 0; index < trimmed.length; index += 1) {
    const char = trimmed[index] ?? "";
    if (quote) {
      result += char;
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === quote) quote = null;
      lineStart = char === "\n";
      continue;
    }
    if (char === "'" || char === '"' || char === "`") {
      quote = char;
      result += char;
      lineStart = false;
      continue;
    }
    const quoteEnd = quoteLikeEnd(trimmed, index);
    if (quoteEnd !== null) {
      result += trimmed.slice(index, quoteEnd + 1);
      index = quoteEnd;
      lineStart = false;
      continue;
    }
    if (char === ";") {
      result += char;
      let next = index + 1;
      while (trimmed[next] === " " || trimmed[next] === "\t") next += 1;
      if (next < trimmed.length && trimmed[next] !== "\n") result += "\n";
      index = next - 1;
      lineStart = true;
      continue;
    }
    if (lineStart && (char === " " || char === "\t")) continue;
    result += char;
    lineStart = char === "\n";
  }
  return result.trimEnd();
}
