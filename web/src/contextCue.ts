import type { SessionContext } from "./sessionContext";

export type ContextCue =
  | { kind: "pending_agent"; provider: string; label: string }
  | { kind: "pending_links"; count: number }
  | { kind: "continued_in"; context: SessionContext };

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
  return delivery.session_id === sessionId ? null : { kind: "continued_in", context };
}
