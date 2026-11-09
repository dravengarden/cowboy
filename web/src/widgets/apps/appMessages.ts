// Messages an app asks to send (docs/widget-apps.md §7). The text never leaves
// the device before the person confirms it: it is checked and shown locally,
// and only the confirmed send reaches the Controller, carrying the digest of
// the exact text and the code identity it was written under. Each
// confirmation is one occurrence, kept in a local outbox from Send until the
// Controller acknowledges it, so a reload retries the same prompt and never
// sends it twice. Fixtures substitute the transport.

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
  /** `sending` survives a reload as an uncertain send to retry or discard. */
  state: "confirming" | "sending";
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

function save(pending: PendingAppMessage): void {
  try {
    localStorage.setItem(OUTBOX + pending.occurrence, JSON.stringify(pending));
  } catch {
    // A full or blocked storage only loses reload recovery, not the send.
  }
}

function forget(pending: PendingAppMessage): void {
  try {
    localStorage.removeItem(OUTBOX + pending.occurrence);
  } catch {}
}

/** Uncertain sends of one app left by an earlier page. */
export function pendingAppMessages(
  sessionId: string,
  widgetId: string,
  nodeId: string,
): PendingAppMessage[] {
  const found: PendingAppMessage[] = [];
  try {
    for (let index = 0; index < localStorage.length; index += 1) {
      const key = localStorage.key(index);
      if (!key?.startsWith(OUTBOX)) continue;
      const value = JSON.parse(localStorage.getItem(key) ?? "null") as PendingAppMessage | null;
      if (
        value &&
        value.sessionId === sessionId &&
        value.widgetId === widgetId &&
        value.nodeId === nodeId
      ) {
        found.push(value);
      }
    }
  } catch {}
  return found;
}

export function discardAppMessage(pending: PendingAppMessage): void {
  forget(pending);
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

/** Sends a confirmed message under its own occurrence. */
export async function sendAppMessage(pending: PendingAppMessage): Promise<AppMessageResult> {
  const sending: PendingAppMessage = { ...pending, state: "sending" };
  save(sending);
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
  const final = status < 500;
  if (final) forget(sending);
  const detail = typeof body["message"] === "string" ? body["message"] : "Not sent. Try again.";
  return { kind: "refused", code, message: REFUSALS[code] ?? detail, final };
}
