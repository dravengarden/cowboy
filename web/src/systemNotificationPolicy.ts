export type SessionNotificationCategory =
  | "completed"
  | "input"
  | "permission"
  | "error";

export type NotificationPermissionState = NotificationPermission | "unsupported";

export interface SystemNotificationPreferences {
  enabled: boolean;
  showSessionNames: boolean;
  categories: Record<SessionNotificationCategory, boolean>;
  mutedSessionIds: string[];
}

export const DEFAULT_SYSTEM_NOTIFICATION_PREFERENCES:
  SystemNotificationPreferences = {
    enabled: false,
    showSessionNames: false,
    categories: { completed: true, input: true, permission: true, error: true },
    mutedSessionIds: [],
  };

export function isSafeSessionId(value: unknown): value is string {
  return typeof value === "string" && /^[A-Za-z0-9_-]{1,160}$/.test(value);
}

export function shouldPresentSessionNotification(input: {
  preferences: SystemNotificationPreferences;
  permission: NotificationPermissionState;
  category: SessionNotificationCategory;
  sessionId: string;
  activeSessionId?: string | undefined;
  visibility: DocumentVisibilityState;
  force?: boolean | undefined;
}): boolean {
  if (input.permission !== "granted" || !input.preferences.enabled) return false;
  if (!input.preferences.categories[input.category]) return false;
  if (input.preferences.mutedSessionIds.includes(input.sessionId)) return false;
  if (input.force) return true;
  return input.visibility !== "visible" || input.sessionId !== input.activeSessionId;
}
