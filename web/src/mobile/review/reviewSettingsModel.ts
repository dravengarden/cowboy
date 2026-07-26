export const REVIEW_CODE_FONT_SIZES = [12, 14, 16, 18] as const;
export const REVIEW_LINE_HEIGHTS = [1.35, 1.5, 1.7] as const;
export const REVIEW_CONTEXT_LINES = [3, 6, 12, -1] as const;

export interface ReviewSettings {
  readonly codeFontSize: number;
  readonly lineHeight: number;
  readonly softWrap: boolean;
  readonly contextLines: number;
  readonly showWhitespaceChanges: boolean;
  readonly diagnostics: boolean;
  readonly inlayHints: boolean;
  readonly semanticHighlighting: boolean;
}

export const DEFAULT_REVIEW_SETTINGS: ReviewSettings = {
  codeFontSize: 14,
  lineHeight: 1.5,
  softWrap: false,
  contextLines: 6,
  showWhitespaceChanges: true,
  diagnostics: true,
  inlayHints: false,
  semanticHighlighting: true,
};

function presetOrDefault(
  value: unknown,
  presets: readonly number[],
  fallback: number,
): number {
  return typeof value === "number" && presets.includes(value)
    ? value
    : fallback;
}

function booleanOrDefault(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

export function normalizeReviewSettings(value: unknown): ReviewSettings {
  const raw = value !== null && typeof value === "object"
    ? value as Partial<ReviewSettings>
    : {};
  return {
    codeFontSize: presetOrDefault(
      raw.codeFontSize,
      REVIEW_CODE_FONT_SIZES,
      DEFAULT_REVIEW_SETTINGS.codeFontSize,
    ),
    lineHeight: presetOrDefault(
      raw.lineHeight,
      REVIEW_LINE_HEIGHTS,
      DEFAULT_REVIEW_SETTINGS.lineHeight,
    ),
    softWrap: booleanOrDefault(
      raw.softWrap,
      DEFAULT_REVIEW_SETTINGS.softWrap,
    ),
    contextLines: presetOrDefault(
      raw.contextLines,
      REVIEW_CONTEXT_LINES,
      DEFAULT_REVIEW_SETTINGS.contextLines,
    ),
    showWhitespaceChanges: booleanOrDefault(
      raw.showWhitespaceChanges,
      DEFAULT_REVIEW_SETTINGS.showWhitespaceChanges,
    ),
    diagnostics: booleanOrDefault(
      raw.diagnostics,
      DEFAULT_REVIEW_SETTINGS.diagnostics,
    ),
    inlayHints: booleanOrDefault(
      raw.inlayHints,
      DEFAULT_REVIEW_SETTINGS.inlayHints,
    ),
    semanticHighlighting: booleanOrDefault(
      raw.semanticHighlighting,
      DEFAULT_REVIEW_SETTINGS.semanticHighlighting,
    ),
  };
}

