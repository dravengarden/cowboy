export type ComposerInputDebugSurface = "desktop" | "mobile";
export type ComposerInputDebugEditor = "cm6" | "textarea";

export interface ComposerInputDebugSample {
  surface: ComposerInputDebugSurface;
  editor: ComposerInputDebugEditor;
  phase: string;
  input_type: string;
  key: string;
  trusted: boolean;
  default_prevented: boolean;
  composing: boolean;
  focus_owned: boolean;
  target_relation: string;
  state_head: number;
  state_line: number;
  document_lines: number;
  line_length: number;
  previous_line_is_image: boolean;
  has_image_widget: boolean;
  caret_anchor_widgets: number;
  native_collapsed: boolean;
  caret_height: number;
  caret_top: number;
  line_height: number;
  line_top: number;
  visual_viewport_height: number;
  dropped_events: number;
}

const SAFE_KEYS = new Set([
  "Enter",
  "Backspace",
  "Delete",
  "Tab",
  "Escape",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "ArrowDown",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "Process",
  "Unidentified",
]);

export const COMPOSER_INPUT_DEBUG_WINDOW_MS = 1000;
export const COMPOSER_INPUT_DEBUG_BUDGET = 24;

export function safeComposerDebugKey(key: string | undefined): string {
  if (!key) return "";
  if (SAFE_KEYS.has(key)) return key;
  if (key.length === 1) return "char";
  return "other";
}

export interface ComposerInputDebugRate {
  windowStart: number;
  windowCount: number;
  dropped: number;
}

export function emptyComposerInputDebugRate(): ComposerInputDebugRate {
  return { windowStart: 0, windowCount: 0, dropped: 0 };
}

export function shouldSampleComposerInputDebug(
  enabled: boolean,
  now: number,
  rate: ComposerInputDebugRate,
): { sample: boolean; dropped: number } {
  if (!enabled) return { sample: false, dropped: 0 };
  if (now - rate.windowStart >= COMPOSER_INPUT_DEBUG_WINDOW_MS) {
    const skipped = rate.dropped;
    rate.windowStart = now;
    rate.windowCount = 0;
    rate.dropped = 0;
    if (skipped > 0) return { sample: true, dropped: skipped };
  }
  if (rate.windowCount >= COMPOSER_INPUT_DEBUG_BUDGET) {
    rate.dropped += 1;
    return { sample: false, dropped: 0 };
  }
  rate.windowCount += 1;
  return { sample: true, dropped: 0 };
}

export function textareaLineMetrics(
  value: string,
  head: number,
): { line: number; lines: number; lineLength: number } {
  const clamped = Math.max(0, Math.min(head, value.length));
  const line = value.slice(0, clamped).split("\n").length;
  const lineStart = value.lastIndexOf("\n", clamped - 1) + 1;
  const lineEnd = value.indexOf("\n", clamped);
  const lineText = value.slice(
    lineStart,
    lineEnd === -1 ? value.length : lineEnd,
  );
  return {
    line,
    lines: value.split("\n").length,
    lineLength: lineText.length,
  };
}
