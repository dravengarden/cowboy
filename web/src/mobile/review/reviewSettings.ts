import { persisted, useStore } from "@cowboy/state-store";
import {
  DEFAULT_REVIEW_SETTINGS,
  loadReviewSettings,
  persistReviewSettings,
  normalizeReviewSettings,
  type ReviewSettings,
} from "./reviewSettingsModel";

const reviewSettingsStore = persisted<ReviewSettings>(
  "cowboy:review-settings",
  DEFAULT_REVIEW_SETTINGS,
  {
    serialize: persistReviewSettings,
    deserialize: (raw) => {
      if (!raw) return DEFAULT_REVIEW_SETTINGS;
      try {
        return loadReviewSettings(JSON.parse(raw));
      } catch {
        return DEFAULT_REVIEW_SETTINGS;
      }
    },
  },
);

export function useReviewSettings(): ReviewSettings {
  return useStore(reviewSettingsStore);
}

export function updateReviewSettings(
  patch: Partial<ReviewSettings>,
): void {
  reviewSettingsStore.set(normalizeReviewSettings({
    ...reviewSettingsStore.get(),
    ...patch,
  }));
}

let reviewLanguage:
  | import("./codeApi").CodeLanguageCapabilities
  | undefined;
const reviewLanguageListeners = new Set<() => void>();
const reviewLanguageStore = {
  get: (): import("./codeApi").CodeLanguageCapabilities | undefined =>
    reviewLanguage,
  subscribe: (listener: () => void): (() => void) => {
    reviewLanguageListeners.add(listener);
    return () => {
      reviewLanguageListeners.delete(listener);
    };
  },
};

export function setReviewLanguageCapabilities(
  next: import("./codeApi").CodeLanguageCapabilities | undefined,
): void {
  reviewLanguage = next;
  for (const listener of reviewLanguageListeners) listener();
}

export function useReviewLanguageCapabilities():
  | import("./codeApi").CodeLanguageCapabilities
  | undefined {
  return useStore(reviewLanguageStore);
}
