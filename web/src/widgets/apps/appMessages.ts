// Messages an app asks to send (docs/widget-apps.md §7): preview, then a
// confirmed send carrying the previewed text's digest. One confirmation is one
// occurrence; it survives a reload so a retried send is the same prompt.
// Fixtures substitute the transport.

export type AppMessageTarget = {
  sessionId: string;
  widgetId: string;
  nodeId: string;
  code: string;
};

export type AppMessageReply =
  | { kind: "preview"; message: string; messageSha256: string; occurrence: string }
  | { kind: "sent" }
  | { kind: "refused"; code: string; message: string };

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

const occurrenceKey = (target: AppMessageTarget, textSha: string): string =>
  `cowboy:app-message:${target.sessionId}:${target.widgetId}:${target.nodeId}:${textSha}`;

function url(target: AppMessageTarget): string {
  const part = encodeURIComponent;
  return `/api/sessions/${part(target.sessionId)}/widgets/${part(target.widgetId)}/apps/${part(target.nodeId)}/messages`;
}

const REFUSALS: Readonly<Record<string, string>> = {
  app_updated: "The agent updated this app, so this message was not sent.",
  closed: "The agent closed this widget.",
  invalid_message: "The app's message contains characters Cowboy will not send.",
  context_pending: "Finish the pending context change first.",
  session_unavailable: "This session cannot take messages now.",
};

function refused(status: number, body: Record<string, unknown>): AppMessageReply {
  const code =
    typeof body["code"] === "string" ? body["code"] : status >= 500 ? "unavailable" : "refused";
  const detail = typeof body["message"] === "string" ? body["message"] : "Not sent. Try again.";
  return { kind: "refused", code, message: REFUSALS[code] ?? detail };
}

/** Asks the Controller to check and preview `text`; reuses the occurrence of
 *  an earlier unsent confirmation of the same text. */
export async function previewAppMessage(
  target: AppMessageTarget,
  text: string,
): Promise<AppMessageReply> {
  const codeSha = await sha256(target.code);
  const key = occurrenceKey(target, await sha256(text));
  const occurrence = sessionStorage.getItem(key) ?? crypto.randomUUID();
  sessionStorage.setItem(key, occurrence);
  try {
    const { status, body } = await transport(url(target), {
      code_sha256: codeSha,
      occurrence,
      text,
    });
    if (status !== 200) return refused(status, body);
    return {
      kind: "preview",
      message: String(body["message"] ?? ""),
      messageSha256: String(body["message_sha256"] ?? ""),
      occurrence,
    };
  } catch {
    return {
      kind: "refused",
      code: "offline",
      message: "Not sent: the connection failed. Try again.",
    };
  }
}

export async function sendAppMessage(
  target: AppMessageTarget,
  preview: Extract<AppMessageReply, { kind: "preview" }>,
): Promise<AppMessageReply> {
  const codeSha = await sha256(target.code);
  try {
    const { status, body } = await transport(url(target), {
      code_sha256: codeSha,
      occurrence: preview.occurrence,
      text: preview.message,
      message_sha256: preview.messageSha256,
    });
    if (status === 200 && body["status"] === "sent") {
      sessionStorage.removeItem(occurrenceKey(target, await sha256(preview.message)));
      return { kind: "sent" };
    }
    return refused(status, body);
  } catch {
    return {
      kind: "refused",
      code: "offline",
      message: "Not sent: the connection failed. Try again.",
    };
  }
}
