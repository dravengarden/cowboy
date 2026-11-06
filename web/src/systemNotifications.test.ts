import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES,
  isSafeSessionId,
  shouldPresentSessionNotification,
} from "./systemNotificationPolicy.ts";
import { normalizePreferences } from "./systemNotifications.ts";

const preferences = structuredClone(DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES);

test("system notifications only interrupt for an unattended session", () => {
  assertEquals(shouldPresentSessionNotification({
    preferences: { ...preferences, enabled: true }, permission: "granted",
    category: "input", sessionId: "sess-1", activeSessionId: "sess-1", visibility: "visible",
  }), false);
  assertEquals(shouldPresentSessionNotification({
    preferences: { ...preferences, enabled: true }, permission: "granted",
    category: "input", sessionId: "sess-2", activeSessionId: "sess-1", visibility: "visible",
  }), true);
  assertEquals(shouldPresentSessionNotification({
    preferences: { ...preferences, enabled: true }, permission: "granted",
    category: "input", sessionId: "sess-1", activeSessionId: "sess-1", visibility: "hidden",
  }), true);
});

test("permission, category, master and per-session mute all gate delivery", () => {
  const base = { category: "error" as const, sessionId: "sess-1", visibility: "hidden" as const };
  assertEquals(shouldPresentSessionNotification({ ...base, preferences, permission: "granted" }), false);
  assertEquals(shouldPresentSessionNotification({ ...base, preferences: { ...preferences, enabled: true }, permission: "denied" }), false);
  assertEquals(shouldPresentSessionNotification({ ...base, preferences: { ...preferences, enabled: true, categories: { ...preferences.categories, error: false } }, permission: "granted" }), false);
  assertEquals(shouldPresentSessionNotification({ ...base, preferences: { ...preferences, enabled: true, mutedSessionIds: ["sess-1"] }, permission: "granted" }), false);
});

test("notification session ids are bounded navigation tokens", () => {
  assertEquals(isSafeSessionId("sess-1786606855067"), true);
  assertEquals(isSafeSessionId("../admin"), false);
  assertEquals(isSafeSessionId(""), false);
});

test("an unversioned preference never opts into completion", () => {
  const legacy = normalizePreferences({
    enabled: true,
    categories: { completed: true, input: true, permission: false, error: true },
  });
  assertEquals(legacy.categories, { completed: false, input: false, permission: false, error: true });
  const chosen = normalizePreferences({ ...legacy, categories: { ...legacy.categories, completed: true } });
  assertEquals(chosen.categories.completed, true, "schema 2 keeps the opt-in");
});
