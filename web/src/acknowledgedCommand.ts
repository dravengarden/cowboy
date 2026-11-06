import type { Envelope, Inbound } from "./protocol";
import { beginConversationClear } from "./conversationClearance";
import { reportClientDuration, startClientSpan } from "./observability";
export function latestClearBoundary(events: readonly Envelope[] = []): number {
  return [...events].reverse().find((event) => event.kind === "update" && event.update.sessionUpdate === "context_cleared")?.seq ?? -1;
}

export function acknowledgedCommand<T>(command: Inbound, predicate: (snapshot: T) => boolean, label: string,
  send: (command: Inbound) => boolean, wait: (predicate: (snapshot: T) => boolean, label: string) => Promise<void>): Promise<void> {
  const operation = startClientSpan("command", { operation: command.type, transport: "websocket" });
  const started = performance.now();
  if (!send(command)) {
    operation?.end("error");
    return Promise.reject(new Error(`${label} is unavailable while reconnecting`));
  }
  if (command.type === "reset_session") beginConversationClear(command.session_id);
  return wait(predicate, label).then(() => {
    operation?.end();
    reportClientDuration("command", performance.now() - started, { operation: command.type, transport: "websocket" });
  }, (error) => { operation?.end("timeout"); throw error; });
}
