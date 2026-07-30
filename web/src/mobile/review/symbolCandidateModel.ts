export interface InspectCandidateScore {
  label: string;
  containsTap: boolean;
  horizontalDistance: number;
  verticalDistance: number;
  rowDistance: number;
  row: number;
  column: number;
}

const KEYWORD_PENALTY = 60;
const CONTAINS_TAP_BONUS = 42;
const SAME_LINE_BONUS = 10;
const VERTICAL_DISTANCE_WEIGHT = 1.35;

const LANGUAGE_KEYWORDS = new Set([
  "abstract",
  "as",
  "async",
  "await",
  "break",
  "case",
  "catch",
  "class",
  "const",
  "continue",
  "def",
  "default",
  "do",
  "else",
  "enum",
  "export",
  "extends",
  "extern",
  "false",
  "final",
  "fn",
  "for",
  "from",
  "function",
  "go",
  "if",
  "impl",
  "import",
  "in",
  "interface",
  "let",
  "match",
  "mod",
  "new",
  "nil",
  "null",
  "package",
  "private",
  "protected",
  "pub",
  "public",
  "return",
  "self",
  "static",
  "struct",
  "super",
  "switch",
  "this",
  "throw",
  "trait",
  "true",
  "try",
  "type",
  "use",
  "var",
  "void",
  "while",
  "with",
  "yield",
]);

export function isLanguageKeyword(label: string): boolean {
  return LANGUAGE_KEYWORDS.has(label);
}

export function rankInspectCandidates<T extends InspectCandidateScore>(
  candidates: readonly T[],
): T[] {
  const score = (candidate: T): number =>
    Math.hypot(
      candidate.horizontalDistance,
      candidate.verticalDistance * VERTICAL_DISTANCE_WEIGHT,
    ) +
    (isLanguageKeyword(candidate.label) ? KEYWORD_PENALTY : 0) -
    (candidate.containsTap ? CONTAINS_TAP_BONUS : 0) -
    (candidate.rowDistance === 0 ? SAME_LINE_BONUS : 0);
  return [...candidates].sort((a, b) => {
    const aKeyword = isLanguageKeyword(a.label);
    const bKeyword = isLanguageKeyword(b.label);
    return score(a) - score(b) ||
      Number(aKeyword) - Number(bKeyword) ||
      a.rowDistance - b.rowDistance ||
      a.horizontalDistance - b.horizontalDistance ||
      a.row - b.row ||
      a.column - b.column;
  });
}

export function rankAndDedupeInspectCandidates<
  T extends InspectCandidateScore,
>(
  candidates: readonly T[],
  limit: number,
): T[] {
  const seen = new Set<string>();
  const ranked: T[] = [];
  for (const candidate of rankInspectCandidates(candidates)) {
    if (seen.has(candidate.label)) continue;
    seen.add(candidate.label);
    ranked.push(candidate);
    if (ranked.length >= limit) break;
  }
  return ranked;
}
