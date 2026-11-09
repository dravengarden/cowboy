// Widget decisions (docs/widgets.md §8, revised): a tap asks the Controller to
// compose the message, the person confirms that exact text, and the confirmed
// send queues it as their ordinary prompt. Fixtures substitute the transport.

import { createContext, useContext } from "react";

/** An input a decision already settled (the Controller's `decided` map). */
export type DecidedInput = { labels: string[]; chosen: boolean; atMs: number };

/** What a published widget's inputs need to act. Absent for inline fences,
 *  whose inputs stay inert. */
export type WidgetInteraction = {
  sessionId: string;
  widgetId: string;
  revision: number;
  closed: boolean;
  decided: Readonly<Record<string, DecidedInput>>;
};

export const WidgetInteractionContext = createContext<WidgetInteraction | null>(null);

export function useWidgetInteraction(): WidgetInteraction | null {
  return useContext(WidgetInteractionContext);
}

export type InteractionRequest = {
  revision: number;
  node_id: string;
  values: string[];
  message_sha256?: string;
};

export type InteractionReply =
  | { kind: "preview"; message: string; messageSha256: string }
  | { kind: "sent"; message: string }
  /** `changed`: the widget changed since the preview; `message` is the new one. */
  | { kind: "changed"; message: string; messageSha256: string }
  | { kind: "refused"; code: string; message: string };

type Transport = (
  sessionId: string,
  widgetId: string,
  request: InteractionRequest,
) => Promise<{ status: number; body: Record<string, unknown> }>;

async function post(
  sessionId: string,
  widgetId: string,
  request: InteractionRequest,
): Promise<{ status: number; body: Record<string, unknown> }> {
  const response = await fetch(
    `/api/sessions/${encodeURIComponent(sessionId)}/widgets/${encodeURIComponent(widgetId)}/interactions`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(request),
    },
  );
  const body = (await response.json().catch(() => ({}))) as Record<string, unknown>;
  return { status: response.status, body };
}

let transport: Transport = post;

/** Fixtures substitute the Controller. */
export function setInteractionTransport(next: Transport | null): void {
  transport = next ?? post;
}

const REFUSALS: Readonly<Record<string, string>> = {
  stale: "The agent updated this widget. Review it and choose again.",
  closed: "The agent closed this widget.",
  decided: "This was already answered.",
  context_pending: "Finish the pending context change first.",
  session_unavailable: "This session cannot take messages now.",
};

export async function requestInteraction(
  sessionId: string,
  widgetId: string,
  request: InteractionRequest,
): Promise<InteractionReply> {
  let reply: { status: number; body: Record<string, unknown> };
  try {
    reply = await transport(sessionId, widgetId, request);
  } catch {
    return {
      kind: "refused",
      code: "offline",
      message: "Not sent: the connection failed. Try again.",
    };
  }
  const { status, body } = reply;
  const text = (key: string): string =>
    typeof body[key] === "string" ? (body[key] as string) : "";
  if (status === 200 && body["status"] === "sent")
    return { kind: "sent", message: text("message") };
  if (status === 200) {
    return { kind: "preview", message: text("message"), messageSha256: text("message_sha256") };
  }
  const code = text("code") || "unavailable";
  if (code === "changed") {
    return { kind: "changed", message: text("message"), messageSha256: text("message_sha256") };
  }
  return {
    kind: "refused",
    code,
    message: REFUSALS[code] ?? (text("message") || "Not sent. Try again."),
  };
}
