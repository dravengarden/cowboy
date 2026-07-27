/**
 * remark-math follows GitHub's `$` / `$$` syntax. Agent output and repository
 * Markdown also commonly use LaTeX's `\(…\)` / `\[…\]` delimiters. Normalize
 * only paired delimiters outside code spans and fenced code blocks so both
 * dialects share the same renderer without changing source or code examples.
 */
export function normalizeMarkdownMath(source: string): string {
  source = normalizePlainTextBoxes(source);
  let output = "";
  let index = 0;
  let lineStart = true;
  let fence: { marker: string; length: number } | undefined;
  let inlineTicks = 0;
  let latexMath: "(" | "[" | undefined;

  while (index < source.length) {
    if (lineStart && !latexMath && inlineTicks === 0) {
      const lineEnd = source.indexOf("\n", index);
      const end = lineEnd < 0 ? source.length : lineEnd;
      const line = source.slice(index, end);
      const match = /^ {0,3}(`{3,}|~{3,})/u.exec(line);
      if (match) {
        const run = match[1]!;
        if (!fence) {
          fence = { marker: run[0]!, length: run.length };
        } else if (
          run[0] === fence.marker &&
          run.length >= fence.length
        ) {
          fence = undefined;
        }
        output += line;
        index = end;
        lineStart = false;
        continue;
      }
    }

    const character = source[index];
    if (character === "\n") {
      output += character;
      index += 1;
      lineStart = true;
      continue;
    }
    lineStart = false;
    if (fence) {
      output += character;
      index += 1;
      continue;
    }

    if (!latexMath && character === "`") {
      let runLength = 1;
      while (source[index + runLength] === "`") runLength += 1;
      if (inlineTicks === 0) inlineTicks = runLength;
      else if (inlineTicks === runLength) inlineTicks = 0;
      output += source.slice(index, index + runLength);
      index += runLength;
      continue;
    }
    if (inlineTicks > 0) {
      output += character;
      index += 1;
      continue;
    }

    const pair = source.slice(index, index + 2);
    const escaped = index > 0 && source[index - 1] === "\\";
    if (!latexMath && !escaped && (pair === "\\[" || pair === "\\(")) {
      const closing = pair === "\\[" ? "\\]" : "\\)";
      if (hasClosingMathDelimiter(source, index + 2, closing)) {
        latexMath = pair === "\\[" ? "[" : "(";
        output += latexMath === "[" ? "$$" : "$";
        index += 2;
        continue;
      }
    } else if (
      latexMath === "[" && pair === "\\]" ||
      latexMath === "(" && pair === "\\)"
    ) {
      output += latexMath === "[" ? "$$" : "$";
      latexMath = undefined;
      index += 2;
      continue;
    }

    output += character;
    index += 1;
  }

  return output;
}

/**
 * Models occasionally use `\boxed{...}` as a prose callout and put Chinese
 * sentences directly inside it. That is not valid TeX: KaTeX renders the whole
 * source in its red error colour, and a long sentence can overflow a phone.
 * Preserve real TeX boxes (including `\boxed{\text{...}}`) while turning only
 * command-free, multi-character prose boxes into a readable Markdown callout.
 */
function normalizePlainTextBoxes(source: string): string {
  return source.replace(
    /\\\[\s*\\boxed\{([\s\S]*?)\}\s*\\\]/gu,
    (match, body: string) => {
      const lines = body.split("\n").map((line) => line.trim()).filter(Boolean);
      const prose = lines.join(" ");
      if (
        prose.length < 16 ||
        /\\[A-Za-z]+/u.test(prose) ||
        !/[\p{Script=Han}，。；：！？]/u.test(prose)
      ) return match;
      return lines.map((line) => `> **${line}**`).join("\n");
    },
  );
}

function hasClosingMathDelimiter(
  source: string,
  start: number,
  closing: "\\]" | "\\)",
): boolean {
  let index = start;
  let lineStart = false;
  let fence: { marker: string; length: number } | undefined;
  let inlineTicks = 0;
  while (index < source.length) {
    if (lineStart && inlineTicks === 0) {
      const lineEnd = source.indexOf("\n", index);
      const end = lineEnd < 0 ? source.length : lineEnd;
      const match = /^ {0,3}(`{3,}|~{3,})/u.exec(source.slice(index, end));
      if (match) {
        const run = match[1]!;
        if (!fence) fence = { marker: run[0]!, length: run.length };
        else if (run[0] === fence.marker && run.length >= fence.length) {
          fence = undefined;
        }
        index = end;
        lineStart = false;
        continue;
      }
    }
    const character = source[index];
    if (character === "\n") {
      index += 1;
      lineStart = true;
      continue;
    }
    lineStart = false;
    if (fence) {
      index += 1;
      continue;
    }
    if (character === "`") {
      let runLength = 1;
      while (source[index + runLength] === "`") runLength += 1;
      if (inlineTicks === 0) inlineTicks = runLength;
      else if (inlineTicks === runLength) inlineTicks = 0;
      index += runLength;
      continue;
    }
    if (
      inlineTicks === 0 &&
      source.slice(index, index + 2) === closing &&
      source[index - 1] !== "\\"
    ) {
      return true;
    }
    index += 1;
  }
  return false;
}
