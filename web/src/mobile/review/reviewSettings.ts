import { persisted, useStore } from "../../_store/mod.ts";
import {
  DEFAULT_REVIEW_SETTINGS,
  normalizeReviewSettings,
  type ReviewSettings,
} from "./reviewSettingsModel";

const reviewSettingsStore = persisted<ReviewSettings>(
  "cowboy:review-settings",
  DEFAULT_REVIEW_SETTINGS,
  {
    serialize: JSON.stringify,
    deserialize: (raw) => {
      if (!raw) return DEFAULT_REVIEW_SETTINGS;
      try {
        return normalizeReviewSettings(JSON.parse(raw));
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

