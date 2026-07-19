/** Reflow parser-normalized jq for a narrow code surface. The parser owns
 * correctness; this pass only chooses display line breaks outside strings. */
export function reflowJq(source: string, columns: number): string {
  const width = Math.max(36, columns - 4);
  const lines: string[] = [];
  let line = "";
  let depth = 0;
  let inString = false;
  let escaped = false;

  const indentation = (): string => "  ".repeat(Math.min(depth, 6));
  const flush = (): void => {
    const value = line.trimEnd();
    if (value.trim()) lines.push(value);
    line = indentation();
  };
  const append = (value: string): void => {
    line += value;
  };
  const isBoundary = (value: string | undefined): boolean =>
    value === undefined || " \t\r\n()[]{},|".includes(value);

  for (let index = 0; index < source.length; index += 1) {
    const char = source[index]!;
    if (inString) {
      append(char);
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') inString = false;
      continue;
    }
    if (char === '"') {
      inString = true;
      append(char);
      continue;
    }
    if (char === "(" || char === "[" || char === "{") {
      append(char);
      depth += 1;
      continue;
    }
    if (char === ")" || char === "]" || char === "}") {
      depth = Math.max(0, depth - 1);
      append(char);
      continue;
    }
    if (char === "|" && source[index - 1] !== "|" && source[index + 1] !== "|") {
      if (depth === 0 || line.trim().length >= Math.floor(width * 0.6)) {
        flush();
        append("| ");
        while (source[index + 1] === " ") index += 1;
      } else {
        line = `${line.trimEnd()} | `;
        while (source[index + 1] === " ") index += 1;
      }
      continue;
    }
    if (char === "," && (line.trim().length >= Math.floor(width * 0.55) || depth <= 1)) {
      append(",");
      flush();
      continue;
    }
    const logical = source.startsWith("and", index) ? "and" : source.startsWith("or", index) ? "or" : "";
    if (
      logical && isBoundary(source[index - 1]) && isBoundary(source[index + logical.length]) &&
      line.trim().length >= Math.floor(width * 0.4)
    ) {
      flush();
      append(`${logical} `);
      index += logical.length - 1;
      while (source[index + 1] === " ") index += 1;
      continue;
    }
    append(char);
    if (line.length > width && char === " ") flush();
  }
  flush();
  return lines.join("\n");
}
