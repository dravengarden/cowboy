// Messages an app asks to send (docs/widget-apps.md §7). The text never leaves
// the device before the person confirms it: it is checked and shown locally,
// and only the confirmed send reaches the Controller, carrying the digest of
// the exact text and the code identity it was written under. Each
// confirmation is one occurrence, saved to a local outbox before it is sent
// and kept until the Controller acknowledges or authoritatively refuses it.
// An uncertain send is retried in the background under the same occurrence
// (so it is never sent twice), whatever became of its widget since.
// Fixtures substitute the transport.

export type AppMessageTarget = {
  sessionId: string;
  widgetId: string;
  nodeId: string;
  code: string;
};

/** One confirmation: frozen when the app asked, sent unchanged. */
export type PendingAppMessage = {
  occurrence: string;
  sessionId: string;
  widgetId: string;
  nodeId: string;
  codeSha256: string;
  text: string;
  textSha256: string;
  /** `sending` survives a reload as an uncertain send to retry. */
  state: "confirming" | "sending";
  /** When the person confirmed it; old uncertain sends are given up. */
  savedAtMs?: number;
};

export type AppMessageResult =
  | { kind: "sent" }
  | { kind: "refused"; code: string; message: string; final: boolean };

type Transport = (
  url: string,
  body: unknown,
) => Promise<{ status: number; body: Record<string, unknown> }>;

const post: Transport = async (url, body) => {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return {
    status: response.status,
    body: (await response.json().catch(() => ({}))) as Record<string, unknown>,
  };
};

let transport: Transport = post;

export function setAppMessageTransport(next: Transport | null): void {
  transport = next ?? post;
}

async function sha256(text: string): Promise<string> {
  const bytes = new Uint8Array(
    await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)),
  );
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

const OUTBOX = "cowboy:app-outbox:";
const MAX_CHARS = 2000;
const MAX_BYTES = 8 * 1024;
// Controls other than newline and tab, and every format character (bidi
// controls, zero-width and invisible ones). The Controller refuses the same.
const HIDDEN = /[\p{Cf}\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u;

/** Why `text` cannot be sent, or null. Matches the Controller's rules. */
export function appMessageProblem(text: string): string | null {
  if (
    !text.trim() ||
    [...text].length > MAX_CHARS ||
    new TextEncoder().encode(text).length > MAX_BYTES
  ) {
    return "The app's message must be 1 to 2,000 characters.";
  }
  if (HIDDEN.test(text)) return "The app's message contains invisible or control characters.";
  return null;
}

/** Freezes a confirmation locally; nothing is sent. */
export async function prepareAppMessage(
  target: AppMessageTarget,
  text: string,
): Promise<PendingAppMessage> {
  return {
    occurrence: crypto.randomUUID(),
    sessionId: target.sessionId,
    widgetId: target.widgetId,
    nodeId: target.nodeId,
    codeSha256: await sha256(target.code),
    text,
    textSha256: await sha256(text),
    state: "confirming",
  };
}

/** Persists `pending`; false if the device would not store it. */
function save(pending: PendingAppMessage): boolean {
  try {
    localStorage.setItem(OUTBOX + pending.occurrence, JSON.stringify(pending));
    return true;
  } catch {
    return false;
  }
}

function forget(pending: PendingAppMessage): void {
  try {
    localStorage.removeItem(OUTBOX + pending.occurrence);
  } catch {}
}

/** Every uncertain send left in this browser's outbox. */
export function pendingAppMessages(): PendingAppMessage[] {
  const found: PendingAppMessage[] = [];
  try {
    for (let index = 0; index < localStorage.length; index += 1) {
      const key = localStorage.key(index);
      if (!key?.startsWith(OUTBOX)) continue;
      const value = JSON.parse(localStorage.getItem(key) ?? "null") as PendingAppMessage | null;
      if (value?.state === "sending") found.push(value);
    }
  } catch {}
  return found;
}

const REFUSALS: Readonly<Record<string, string>> = {
  app_updated: "The agent updated this app, so this message was not sent.",
  closed: "The agent closed this widget.",
  invalid_message: "The app's message contains characters Cowboy will not send.",
  context_pending: "Finish the pending context change first.",
  session_unavailable: "This session cannot take messages now.",
};

function url(pending: PendingAppMessage): string {
  const part = encodeURIComponent;
  return `/api/sessions/${part(pending.sessionId)}/widgets/${part(pending.widgetId)}/apps/${part(pending.nodeId)}/messages`;
}

// Refusals that may pass (sign-in, rate limits, a pending context change)
// keep the outbox entry; the others are the Controller's final answer.
const TRANSIENT_STATUS = new Set([401, 403, 408, 425, 429]);
const TRANSIENT_CODES = new Set(["context_pending"]);

// One request per occurrence at a time: the confirmation and background
// recovery share it, so neither reports a result the other contradicts.
const inFlight = new Map<string, Promise<AppMessageResult>>();

/** Sends a confirmed message under its own occurrence. Nothing is sent
 *  unless the outbox entry was saved first. */
export function sendAppMessage(pending: PendingAppMessage): Promise<AppMessageResult> {
  const running = inFlight.get(pending.occurrence);
  if (running) return running;
  const attempt = transmit(pending).finally(() => inFlight.delete(pending.occurrence));
  inFlight.set(pending.occurrence, attempt);
  return attempt;
}

async function transmit(pending: PendingAppMessage): Promise<AppMessageResult> {
  const sending: PendingAppMessage = {
    ...pending,
    state: "sending",
    savedAtMs: pending.savedAtMs ?? Date.now(),
  };
  if (!save(sending)) {
    return {
      kind: "refused",
      code: "storage",
      message: "Not sent: this browser would not save the message first. Free some storage and try again.",
      final: false,
    };
  }
  let reply: { status: number; body: Record<string, unknown> };
  try {
    reply = await transport(url(sending), {
      code_sha256: sending.codeSha256,
      occurrence: sending.occurrence,
      text: sending.text,
      message_sha256: sending.textSha256,
    });
  } catch {
    // Uncertain: keep the outbox entry so a retry is the same prompt.
    return {
      kind: "refused",
      code: "offline",
      message: "Not confirmed: the connection failed. Try again.",
      final: false,
    };
  }
  const { status, body } = reply;
  if (status === 200 && body["status"] === "sent") {
    forget(sending);
    return { kind: "sent" };
  }
  const code =
    typeof body["code"] === "string" ? body["code"] : status >= 500 ? "unavailable" : "refused";
  const final = status < 500 && !TRANSIENT_STATUS.has(status) && !TRANSIENT_CODES.has(code);
  if (final) forget(sending);
  const detail = typeof body["message"] === "string" ? body["message"] : "Not sent. Try again.";
  return { kind: "refused", code, message: REFUSALS[code] ?? detail, final };
}

const GIVE_UP_MS = 7 * 24 * 60 * 60 * 1000;
const RETRY_GAP_MS = 30_000;
let retrying = false;
let lastRetry = 0;
let retryTimer: ReturnType<typeof setTimeout> | null = null;

/** Retries every uncertain send; final answers and old entries leave the
 *  outbox. A request that comes too soon after the last attempt, or entries
 *  still left afterwards, schedule the next attempt instead of being lost. */
export async function retryAppMessages(): Promise<void> {
  if (retrying) return;
  const wait = lastRetry + RETRY_GAP_MS - Date.now();
  if (wait > 0) return scheduleRetry(wait);
  retrying = true;
  lastRetry = Date.now();
  try {
    for (const pending of pendingAppMessages()) {
      if (pending.savedAtMs === undefined) {
        // Saved before entries carried a time: start its clock now.
        save({ ...pending, savedAtMs: Date.now() });
        await sendAppMessage(pending);
      } else if (Date.now() - pending.savedAtMs > GIVE_UP_MS) forget(pending);
      else await sendAppMessage(pending);
    }
  } finally {
    retrying = false;
  }
  if (pendingAppMessages().length > 0) scheduleRetry(RETRY_GAP_MS);
}

function scheduleRetry(ms: number): void {
  if (retryTimer !== null) return;
  retryTimer = setTimeout(() => {
    retryTimer = null;
    if (document.visibilityState === "visible") void retryAppMessages();
  }, ms);
}

/** Retries uncertain sends after load, on reconnect and when the page shows. */
export function installAppMessageRecovery(): void {
  if (typeof window === "undefined") return;
  const retry = (): void => {
    if (document.visibilityState === "visible") void retryAppMessages();
  };
  setTimeout(retry, 3000);
  window.addEventListener("online", retry);
  document.addEventListener("visibilitychange", retry);
}
