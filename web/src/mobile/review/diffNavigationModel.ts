export function diffHunkLines(text: string): number[] {
  const lines: number[] = [];
  let lineNumber = 1;
  for (const line of text.split("\n")) {
    if (line.startsWith("@@")) lines.push(lineNumber);
    lineNumber += 1;
  }
  return lines;
}

export function reviewEntryKey(path: string, scope: string): string {
  return `${scope}\0${path}`;
}
