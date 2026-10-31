import type { SessionContext } from "./sessionContext";

export type ContextCue =
  | { kind: "pending_agent"; provider: string; label: string }
  | { kind: "pending_links"; count: number }
  | { kind: "continued_in"; context: SessionContext }
  | {
    kind: "continued_from";
    id: string;
    title: string;
    more: number;
    label?: string;
    before?: number;
  };

export function contextCue(
  sessionId: string,
  context: SessionContext | undefined,
  label: (provider: string, account: string) => string,
): ContextCue | null {
  if (!context) return null;
  const delivery = context.delivery;
  if (delivery?.state === "preparing" || delivery?.state === "unknown") {
    return null; // The Context editor owns resolution of an uncertain send.
  }
  if (context.revision !== context.applied_revision) {
    if (context.next_agent) {
      return {
        kind: "pending_agent",
        provider: context.next_agent.provider,
        label: label(
          context.next_agent.provider,
          context.next_agent.provider_account_id,
        ),
      };
    }
    return context.sources.length > 0
      ? { kind: "pending_links", count: context.sources.length }
      : null;
  }
  if (delivery?.state !== "accepted") return null;
  if (delivery.session_id !== sessionId) {
    return { kind: "continued_in", context };
  }
  const sources = context.source_sessions.filter((entry) =>
    entry.id !== sessionId && entry.available
  );
  const source = sources[0];
  return source
    ? {
      kind: "continued_from",
      id: source.id,
      title: source.title || "a linked session",
      more: sources.length - 1,
      ...(source.provider && {
        label: label(source.provider, source.provider_account_id ?? "default"),
      }),
      ...(typeof source.before === "number" && { before: source.before }),
    }
    : null;
}
