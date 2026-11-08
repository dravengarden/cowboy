export interface DiffContextFold {
  fromLine: number;
  toLine: number;
  hiddenLines: number;
}

export function diffContextFolds(
  text: string,
  keepLines = 3,
  minimumRun = 12,
): DiffContextFold[] {
  const lines = text.split("\n");
  const folds: DiffContextFold[] = [];
  let start = -1;

  const flush = (endExclusive: number): void => {
    if (start < 0) return;
    const length = endExclusive - start;
    if (length >= minimumRun && length > keepLines * 2) {
      const from = start + keepLines;
      const to = endExclusive - keepLines - 1;
      folds.push({
        fromLine: from + 1,
        toLine: to + 1,
        hiddenLines: to - from + 1,
      });
    }
    start = -1;
  };

  lines.forEach((line, index) => {
    if (line.startsWith(" ")) {
      if (start < 0) start = index;
    } else {
      flush(index);
    }
  });
  flush(lines.length);
  return folds;
}
