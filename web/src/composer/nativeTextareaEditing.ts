export interface NativeTextEdit {
  value: string;
  from: number;
  to: number;
}

/**
 * Ignore the subpixel scrollHeight drift WebKit reports for a fitted textarea.
 * Turning that one-pixel rounding residue into a real scroll container sends
 * iOS caret geometry through its stale subscroll-offset path after Return.
 */
export function nativeTextareaNeedsScroll(
  scrollHeight: number,
  clientHeight: number,
): boolean {
  return scrollHeight - clientHeight > 2;
}

/** Compact native field height. Grow with text; never call setSelectionRange. */
export const NATIVE_TEXTAREA_MIN_HEIGHT_PX = 48;

export function nativeTextareaFittedHeight(
  scrollHeight: number,
  minHeight = NATIVE_TEXTAREA_MIN_HEIGHT_PX,
): number {
  return Math.max(minHeight, scrollHeight);
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

/** Replace one native textarea range and leave a collapsed caret after it. */
export function replaceNativeSelection(
  value: string,
  from: number,
  to: number,
  insert: string,
): NativeTextEdit {
  const [start, end] = orderedSelection(value, from, to);
  const caret = start + insert.length;
  return {
    value: value.slice(0, start) + insert + value.slice(end),
    from: caret,
    to: caret,
  };
}

/**
 * Map a native textarea selection through an external value replacement.
 *
 * Normal typing never calls this: the DOM value and selection stay owned by the
 * browser. It is for genuine outside changes such as clearing a draft or loading
 * another one. Treat the smallest differing span as the replacement and keep the
 * caret after that span, so a newline or prefix inserted by a toolbar does not
 * strand it at the old coordinate.
 */
export function mapNativeSelectionThroughValueChange(
  previousValue: string,
  nextValue: string,
  from: number,
  to: number,
): { from: number; to: number } {
  const [oldFrom, oldTo] = orderedSelection(previousValue, from, to);
  let prefix = 0;
  while (
    prefix < previousValue.length && prefix < nextValue.length &&
    previousValue[prefix] === nextValue[prefix]
  ) {
    prefix += 1;
  }
  let suffix = 0;
  while (
    suffix < previousValue.length - prefix &&
    suffix < nextValue.length - prefix &&
    previousValue[previousValue.length - suffix - 1] ===
      nextValue[nextValue.length - suffix - 1]
  ) {
    suffix += 1;
  }
  const oldChangeEnd = previousValue.length - suffix;
  const newChangeEnd = nextValue.length - suffix;
  const delta = newChangeEnd - oldChangeEnd;
  const mapPosition = (position: number): number => {
    if (position < prefix) return position;
    if (position >= oldChangeEnd) return position + delta;
    return newChangeEnd;
  };
  return {
    from: Math.max(0, Math.min(mapPosition(oldFrom), nextValue.length)),
    to: Math.max(0, Math.min(mapPosition(oldTo), nextValue.length)),
  };
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
