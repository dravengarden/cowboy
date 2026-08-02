export interface NativeTextEdit {
  value: string;
  from: number;
  to: number;
}

function orderedSelection(
  value: string,
  from: number,
  to: number,
): [number, number] {
  const length = value.length;
  const a = Math.max(0, Math.min(from, length));
  const b = Math.max(0, Math.min(to, length));
  return a <= b ? [a, b] : [b, a];
}

function lineStart(value: string, position: number): number {
  return value.lastIndexOf("\n", Math.max(0, position) - 1) + 1;
}

function lineEnd(value: string, position: number): number {
  const next = value.indexOf("\n", position);
  return next < 0 ? value.length : next;
}

export function wrapNativeSelection(
  value: string,
  from: number,
  to: number,
  before: string,
  after: string,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const selected = value.slice(start, end);
  return {
    value: value.slice(0, start) + before + selected + after + value.slice(end),
    from: start + before.length,
    to: start === end
      ? start + before.length
      : start + before.length + selected.length,
  };
}

export function toggleNativeWrap(
  value: string,
  from: number,
  to: number,
  marker: string,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const selected = value.slice(start, end);
  const markerLength = marker.length;
  if (
    selected.length >= markerLength * 2 &&
    selected.startsWith(marker) &&
    selected.endsWith(marker)
  ) {
    const inner = selected.slice(markerLength, -markerLength);
    return {
      value: value.slice(0, start) + inner + value.slice(end),
      from: start,
      to: start + inner.length,
    };
  }
  const outerStart = start - markerLength;
  const outerEnd = end + markerLength;
  if (
    outerStart >= 0 && outerEnd <= value.length &&
    value.slice(outerStart, start) === marker &&
    value.slice(end, outerEnd) === marker
  ) {
    return {
      value: value.slice(0, outerStart) + selected + value.slice(outerEnd),
      from: outerStart,
      to: outerStart + selected.length,
    };
  }
  return wrapNativeSelection(value, start, end, marker, marker);
}

function replaceCurrentLinePrefix(
  value: string,
  from: number,
  to: number,
  removeLength: number,
  insert: string,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const at = lineStart(value, end);
  const delta = insert.length - removeLength;
  return {
    value: value.slice(0, at) + insert + value.slice(at + removeLength),
    from: Math.max(at, start + delta),
    to: Math.max(at, end + delta),
  };
}

export function toggleNativeLinePrefix(
  value: string,
  from: number,
  to: number,
  prefix: string,
): NativeTextEdit {
  const [, end] = orderedSelection(value, from, to);
  const at = lineStart(value, end);
  const hasPrefix = value.startsWith(prefix, at);
  return replaceCurrentLinePrefix(
    value,
    from,
    to,
    hasPrefix ? prefix.length : 0,
    hasPrefix ? "" : prefix,
  );
}

export function setNativeHeading(
  value: string,
  from: number,
  to: number,
  level: number,
): NativeTextEdit {
  const [, end] = orderedSelection(value, from, to);
  const at = lineStart(value, end);
  const current = /^(#{1,6})\s/u.exec(value.slice(at, lineEnd(value, end)));
  const removeLength = current?.[0]?.length ?? 0;
  const insert = level <= 0 ? "" : `${"#".repeat(Math.min(level, 6))} `;
  return replaceCurrentLinePrefix(value, from, to, removeLength, insert);
}

export function cycleNativeHeading(
  value: string,
  from: number,
  to: number,
): NativeTextEdit {
  const [, end] = orderedSelection(value, from, to);
  const at = lineStart(value, end);
  const current = /^(#{1,6})\s/u.exec(value.slice(at, lineEnd(value, end)));
  const level = current?.[1]?.length ?? 0;
  return setNativeHeading(value, from, to, level >= 3 ? 0 : level + 1);
}

export function toggleNativeCheckbox(
  value: string,
  from: number,
  to: number,
): NativeTextEdit | null {
  const [start, end] = orderedSelection(value, from, to);
  const at = lineStart(value, end);
  const match = /^(\s*[-*+]\s+)\[([ xX])\]/u.exec(
    value.slice(at, lineEnd(value, end)),
  );
  if (match?.[1] === undefined || match[2] === undefined) return null;
  const boxAt = at + match[1].length + 1;
  const checked = match[2] !== " ";
  return {
    value: value.slice(0, boxAt) + (checked ? " " : "x") +
      value.slice(boxAt + 1),
    from: start,
    to: end,
  };
}

interface LineChange {
  at: number;
  remove: number;
  insert: string;
}

function applyLineChanges(
  value: string,
  from: number,
  to: number,
  changes: LineChange[],
): NativeTextEdit {
  let next = value;
  for (const change of [...changes].reverse()) {
    next = next.slice(0, change.at) + change.insert +
      next.slice(change.at + change.remove);
  }
  const mapPosition = (position: number): number => {
    let delta = 0;
    for (const change of changes) {
      if (position < change.at) break;
      if (position <= change.at + change.remove) {
        return change.at + delta + change.insert.length;
      }
      delta += change.insert.length - change.remove;
    }
    return position + delta;
  };
  return { value: next, from: mapPosition(from), to: mapPosition(to) };
}

function selectedLineStarts(value: string, from: number, to: number): number[] {
  const [start, end] = orderedSelection(value, from, to);
  const starts = [lineStart(value, start)];
  const effectiveEnd = end > start && value[end - 1] === "\n" ? end - 1 : end;
  let cursor = value.indexOf("\n", starts[0]);
  while (cursor >= 0 && cursor < effectiveEnd) {
    starts.push(cursor + 1);
    cursor = value.indexOf("\n", cursor + 1);
  }
  return starts;
}

export function indentNativeLines(
  value: string,
  from: number,
  to: number,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const changes = selectedLineStarts(value, start, end).map((at) => ({
    at,
    remove: 0,
    insert: "  ",
  }));
  return applyLineChanges(value, start, end, changes);
}

export function outdentNativeLines(
  value: string,
  from: number,
  to: number,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const changes = selectedLineStarts(value, start, end).flatMap((at) => {
    const indent = /^(?: {1,2}|\t)/u.exec(value.slice(at))?.[0] ?? "";
    return indent === "" ? [] : [{ at, remove: indent.length, insert: "" }];
  });
  return applyLineChanges(value, start, end, changes);
}

export function insertNativeLink(
  value: string,
  from: number,
  to: number,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const label = value.slice(start, end) || "text";
  const markdown = `[${label}](url)`;
  const urlAt = start + label.length + 3;
  return {
    value: value.slice(0, start) + markdown + value.slice(end),
    from: urlAt,
    to: urlAt + 3,
  };
}

export function insertNativeCodeBlock(
  value: string,
  from: number,
  to: number,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const selected = value.slice(start, end);
  const markdown = `\`\`\`\n${selected}\n\`\`\``;
  const caret = start + 4 + selected.length;
  return {
    value: value.slice(0, start) + markdown + value.slice(end),
    from: caret,
    to: caret,
  };
}
