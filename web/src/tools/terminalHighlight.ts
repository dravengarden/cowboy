export interface TerminalHighlightSegment {
  text: string;
  language?: string;
}

const MAX_STRUCTURED_OUTPUT = 128 * 1024;

function parsesJson(text: string): boolean {
  try {
    JSON.parse(text);
    return true;
  } catch {
    return false;
  }
}

function wholeLanguage(text: string): string | undefined {
  const trimmed = text.trim();
  if (!trimmed) return undefined;
  if ((trimmed.startsWith("{") || trimmed.startsWith("[")) && parsesJson(trimmed)) return "json";
  if (/^(?:diff --git |@@ |--- a\/|\+\+\+ b\/)/m.test(trimmed)) return "diff";
  if (/^<\?xml\b|^<[A-Za-z][^>]*>[\s\S]*<\/[A-Za-z][^>]*>$/m.test(trimmed)) return "markup";
  if (/^#!.*\b(?:ba|z|k)?sh\b/.test(trimmed)) return "bash";
  if (/^(?:SELECT|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP|WITH)\b/i.test(trimmed)) return "sql";
  return undefined;
}

/** Find a complete JSON value without interpreting braces inside strings. */
function jsonEnd(text: string, start: number): number | undefined {
  const opener = text[start];
  if (opener !== "{" && opener !== "[") return undefined;
  const stack = [opener];
  let quoted = false;
  let escaped = false;
  for (let index = start + 1; index < text.length; index++) {
    const char = text[index];
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') quoted = false;
      continue;
    }
    if (char === '"') quoted = true;
    else if (char === "{" || char === "[") stack.push(char);
    else if (char === "}" || char === "]") {
      const expected = char === "}" ? "{" : "[";
      if (stack.pop() !== expected) return undefined;
      if (stack.length === 0) return index + 1;
    }
  }
  return undefined;
}

/**
 * Conservative, display-only structure detection for terminal output.
 * It never mutates source bytes: confidently recognised islands are highlighted,
 * and anything ambiguous remains plain terminal text.
 */
export function terminalHighlightSegments(text: string): TerminalHighlightSegment[] {
  if (!text || text.length > MAX_STRUCTURED_OUTPUT) return [{ text }];
  const language = wholeLanguage(text);
  if (language) return [{ text, language }];

  const segments: TerminalHighlightSegment[] = [];
  let cursor = 0;
  let islands = 0;
  const lineValue = /^(\s*)([{[])/gm;
  for (let match = lineValue.exec(text); match && islands < 16; match = lineValue.exec(text)) {
    const start = match.index + (match[1] ?? "").length;
    if (start < cursor) continue;
    const end = jsonEnd(text, start);
    if (end === undefined) continue;
    const candidate = text.slice(start, end);
    if (!parsesJson(candidate)) continue;
    if (start > cursor) segments.push({ text: text.slice(cursor, start) });
    segments.push({ text: candidate, language: "json" });
    cursor = end;
    islands++;
    lineValue.lastIndex = end;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor) });
  return segments.length > 1 ? segments : [{ text }];
}

/** Rendering may separate structured islands into cards; whitespace-only
 * separators must not become empty cards between them. */
export function terminalDisplaySegments(text: string): TerminalHighlightSegment[] {
  return terminalHighlightSegments(text).filter((segment) => segment.text.trim().length > 0);
}
