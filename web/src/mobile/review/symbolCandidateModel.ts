export interface InspectCandidateScore {
  label: string;
  containsTap: boolean;
  distance: number;
  column: number;
}

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
  return [...candidates].sort((a, b) => {
    const aKeyword = isLanguageKeyword(a.label);
    const bKeyword = isLanguageKeyword(b.label);
    return Number(aKeyword) - Number(bKeyword) ||
      Number(b.containsTap) - Number(a.containsTap) ||
      a.distance - b.distance ||
      a.column - b.column;
  });
}
