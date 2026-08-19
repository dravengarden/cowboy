import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES,
  isSafeSessionId,
  shouldPresentSessionNotification,
} from "./systemNotificationPolicy.ts";

const preferences = structuredClone(DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES);

Deno.test("system notifications only interrupt for an unattended session", () => {
  assertEquals(shouldPresentSessionNotification({
    preferences: { ...preferences, enabled: true }, permission: "granted",
    category: "completed", sessionId: "sess-1", activeSessionId: "sess-1", visibility: "visible",
  }), false);
  assertEquals(shouldPresentSessionNotification({
    preferences: { ...preferences, enabled: true }, permission: "granted",
    category: "completed", sessionId: "sess-2", activeSessionId: "sess-1", visibility: "visible",
  }), true);
  assertEquals(shouldPresentSessionNotification({
    preferences: { ...preferences, enabled: true }, permission: "granted",
    category: "completed", sessionId: "sess-1", activeSessionId: "sess-1", visibility: "hidden",
  }), true);
});

Deno.test("permission, category, master and per-session mute all gate delivery", () => {
  const base = { category: "error" as const, sessionId: "sess-1", visibility: "hidden" as const };
  assertEquals(shouldPresentSessionNotification({ ...base, preferences, permission: "granted" }), false);
  assertEquals(shouldPresentSessionNotification({ ...base, preferences: { ...preferences, enabled: true }, permission: "denied" }), false);
  assertEquals(shouldPresentSessionNotification({ ...base, preferences: { ...preferences, enabled: true, categories: { ...preferences.categories, error: false } }, permission: "granted" }), false);
  assertEquals(shouldPresentSessionNotification({ ...base, preferences: { ...preferences, enabled: true, mutedSessionIds: ["sess-1"] }, permission: "granted" }), false);
});

Deno.test("notification session ids are bounded navigation tokens", () => {
  assertEquals(isSafeSessionId("sess-1786606855067"), true);
  assertEquals(isSafeSessionId("../admin"), false);
  assertEquals(isSafeSessionId(""), false);
});
