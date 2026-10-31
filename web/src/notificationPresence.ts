// Tells the Controller which session this device is looking at, so a failure or
// completion waits rather than alerts while the person is there. A lease that
// stops renewing expires on its own; any failure here only means a notification
// arrives (docs/task-notifications.md#presence).

const RENEW_MS = 15_000;
const DEVICE_KEY = "cowboy:notification-device";

function deviceId(): string {
  const stored = globalThis.localStorage?.getItem(DEVICE_KEY);
  if (stored) return stored;
  const id = globalThis.crypto.randomUUID();
  globalThis.localStorage?.setItem(DEVICE_KEY, id);
  return id;
}

function request(method: "PUT" | "DELETE", body: Record<string, string>): void {
  void fetch("/api/notifications/presence", {
    method,
    keepalive: true,
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  }).catch(() => undefined);
}

/** Start reporting; the returned function reports a change right away. */
export function startNotificationPresence(current: () => string | null): () => void {
  if (typeof globalThis.addEventListener !== "function" || typeof globalThis.fetch !== "function") {
    return () => undefined;
  }
  let held: string | null = null;
  const publish = (): void => {
    const session = current();
    if (session) {
      request("PUT", { device_id: deviceId(), session_id: session });
    } else if (held) {
      request("DELETE", { device_id: deviceId() });
    }
    held = session;
  };
  globalThis.addEventListener("focus", publish);
  globalThis.addEventListener("blur", publish);
  globalThis.addEventListener("hashchange", publish);
  globalThis.document?.addEventListener("visibilitychange", publish);
  globalThis.setInterval(publish, RENEW_MS);
  return publish;
}
