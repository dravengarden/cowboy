export const REVIEW_CONTEXT_LINES = [3, 6, 12, -1] as const;
export const REVIEW_CODE_FONT_SIZES = [6, 7, 8, 10, 12, 14, 16, 18] as const;

export interface ReviewSettings {
  readonly codeFontSize: number;
  readonly softWrap: boolean;
  readonly contextLines: number;
  readonly showWhitespaceChanges: boolean;
  readonly diagnostics: boolean;
  readonly inlayHints: boolean;
  readonly semanticHighlighting: boolean;
}

export const DEFAULT_REVIEW_SETTINGS: ReviewSettings = {
  codeFontSize: 8,
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

/** Old factory size. Devices that never picked a font still store 14. */
export const LEGACY_DEFAULT_CODE_FONT_SIZE = 14;
export const REVIEW_CODE_FONT_DEFAULT_GENERATION = 2;

/** Move the untouched factory size to the new default. An explicit 12/16 stays. */
export function migrateReviewCodeFontSize(size: number): number {
  return size === LEGACY_DEFAULT_CODE_FONT_SIZE
    ? DEFAULT_REVIEW_SETTINGS.codeFontSize
    : size;
}

export function loadReviewSettings(value: unknown): ReviewSettings {
  const raw = value !== null && typeof value === "object"
    ? value as Partial<ReviewSettings> & { codeFontDefaultGeneration?: number }
    : {};
  const settings = normalizeReviewSettings(raw);
  if (raw.codeFontDefaultGeneration === REVIEW_CODE_FONT_DEFAULT_GENERATION) {
    return settings;
  }
  return {
    ...settings,
    codeFontSize: migrateReviewCodeFontSize(settings.codeFontSize),
  };
}

export function persistReviewSettings(settings: ReviewSettings): string {
  return JSON.stringify({
    ...settings,
    codeFontDefaultGeneration: REVIEW_CODE_FONT_DEFAULT_GENERATION,
  });
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
