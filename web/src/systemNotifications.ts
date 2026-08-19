import { persisted, useStore } from "./_store/mod.ts";
import {
  DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES,
  isSafeSessionId,
  type NotificationPermissionState,
  type SessionNotificationCategory,
  type SystemNotificationPreferences,
} from "./systemNotificationPolicy.ts";
export type {
  NotificationPermissionState,
  SessionNotificationCategory,
  SystemNotificationPreferences,
} from "./systemNotificationPolicy.ts";

function normalizePreferences(value: unknown): SystemNotificationPreferences {
  const candidate = value && typeof value === "object"
    ? value as Partial<SystemNotificationPreferences>
    : {};
  const categories = candidate.categories && typeof candidate.categories === "object"
    ? candidate.categories as Partial<Record<SessionNotificationCategory, boolean>>
    : {};
  return {
    enabled: candidate.enabled === true,
    showSessionNames: candidate.showSessionNames === true,
    categories: {
      completed: categories.completed !== false,
      input: categories.input !== false,
      permission: categories.permission !== false,
      error: categories.error !== false,
    },
    mutedSessionIds: Array.isArray(candidate.mutedSessionIds)
      ? [...new Set(candidate.mutedSessionIds.filter(isSafeSessionId))]
      : [],
  };
}

const preferences = persisted<SystemNotificationPreferences>(
  "cowboy:system-notifications",
  DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES,
  {
    serialize: JSON.stringify,
    deserialize: (raw) => normalizePreferences(JSON.parse(raw)),
  },
);

export function useSystemNotificationPreferences(): SystemNotificationPreferences {
  return useStore(preferences);
}

export function getSystemNotificationPreferences(): SystemNotificationPreferences {
  return preferences.get();
}

export function updateSystemNotificationPreferences(
  update: Partial<Omit<SystemNotificationPreferences, "categories" | "mutedSessionIds">> & {
    categories?: Partial<Record<SessionNotificationCategory, boolean>>;
    mutedSessionIds?: string[];
  },
): void {
  preferences.set((current) => normalizePreferences({
    ...current,
    ...update,
    categories: { ...current.categories, ...update.categories },
  }));
  if (preferences.get().enabled) void syncWebPushSubscription();
}

export function sessionNotificationsMuted(sessionId: string): boolean {
  return preferences.get().mutedSessionIds.includes(sessionId);
}

export function setSessionNotificationsMuted(sessionId: string, muted: boolean): void {
  if (!isSafeSessionId(sessionId)) return;
  const current = preferences.get();
  const ids = new Set(current.mutedSessionIds);
  if (muted) ids.add(sessionId);
  else ids.delete(sessionId);
  updateSystemNotificationPreferences({ mutedSessionIds: [...ids] });
}

export function notificationPermissionState(): NotificationPermissionState {
  return typeof globalThis.Notification === "undefined" ||
      !("serviceWorker" in globalThis.navigator) ||
      typeof globalThis.PushManager === "undefined"
    ? "unsupported"
    : globalThis.Notification.permission;
}

export async function requestSystemNotificationPermission(): Promise<NotificationPermissionState> {
  if (typeof globalThis.Notification === "undefined") return "unsupported";
  const result = await globalThis.Notification.requestPermission();
  if (result !== "granted") {
    preferences.set((current) => ({ ...current, enabled: false }));
    return result;
  }
  try {
    const response = await fetch("/api/web-push/config", {
      cache: "no-store",
      headers: { Accept: "application/json" },
    });
    if (!response.ok) throw new Error("Notification service is unavailable.");
    const config = await response.json() as { applicationServerKey?: unknown };
    if (typeof config.applicationServerKey !== "string") {
      throw new Error("Notification service returned an invalid key.");
    }
    const registration = await globalThis.navigator.serviceWorker.ready;
    let subscription = await registration.pushManager.getSubscription();
    subscription ??= await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: decodeApplicationServerKey(config.applicationServerKey),
    });
    await putSubscription(subscription, preferences.get());
    preferences.set((current) => ({ ...current, enabled: true }));
  } catch (error) {
    preferences.set((current) => ({ ...current, enabled: false }));
    throw error;
  }
  return result;
}

export async function disableSystemNotifications(): Promise<void> {
  const registration = await globalThis.navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.getSubscription();
  if (subscription) {
    try {
      await fetch("/api/web-push/subscription", {
        method: "DELETE",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ endpoint: subscription.endpoint }),
      });
    } catch {
      // Invalidating the browser endpoint still stops delivery. The server
      // removes its stale copy after the Push Service returns 404/410.
    }
    await subscription.unsubscribe();
  }
  preferences.set((current) => ({ ...current, enabled: false }));
}

export async function reconcileSystemNotificationSubscription(): Promise<boolean> {
  if (notificationPermissionState() !== "granted") {
    preferences.set((current) => current.enabled ? { ...current, enabled: false } : current);
    return false;
  }
  const registration = await globalThis.navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.getSubscription();
  const subscribed = subscription !== null;
  if (subscription) await putSubscription(subscription, preferences.get());
  preferences.set((current) => current.enabled === subscribed ? current : { ...current, enabled: subscribed });
  return subscribed;
}

function decodeApplicationServerKey(value: string): Uint8Array<ArrayBuffer> {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/")
    .padEnd(Math.ceil(value.length / 4) * 4, "=");
  const bytes = Uint8Array.from(globalThis.atob(padded), (character) => character.charCodeAt(0));
  if (bytes.length !== 65) throw new Error("Invalid application server key.");
  return bytes;
}

async function putSubscription(
  subscription: PushSubscription,
  prefs: SystemNotificationPreferences,
): Promise<void> {
  const json = subscription.toJSON();
  if (!json.endpoint || !json.keys?.p256dh || !json.keys?.auth) {
    throw new Error("The browser returned an incomplete notification subscription.");
  }
  const response = await fetch("/api/web-push/subscription", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      endpoint: json.endpoint,
      keys: { p256dh: json.keys.p256dh, auth: json.keys.auth },
      preferences: {
        showSessionNames: prefs.showSessionNames,
        categories: prefs.categories,
        mutedSessionIds: prefs.mutedSessionIds,
      },
    }),
  });
  if (!response.ok) throw new Error("Cowboy could not save this notification subscription.");
}

async function syncWebPushSubscription(): Promise<void> {
  try {
    const registration = await globalThis.navigator.serviceWorker.ready;
    const subscription = await registration.pushManager.getSubscription();
    if (subscription) await putSubscription(subscription, preferences.get());
  } catch {
    // Keep local preferences; a later Settings open or edit retries the sync.
  }
}

export async function presentTestNotification(): Promise<boolean> {
  if (notificationPermissionState() !== "granted") return false;
  try {
    const registration = await globalThis.navigator.serviceWorker.ready;
    const worker = registration.active ?? registration.waiting;
    if (!worker) return false;
    worker.postMessage({
      type: "cowboy.session-notification",
      version: 1,
      category: "completed",
      sessionId: "notification-test",
      title: "Cowboy notifications work",
      body: "You will be alerted when a session needs attention.",
      test: true,
    });
    return true;
  } catch {
    return false;
  }
}
