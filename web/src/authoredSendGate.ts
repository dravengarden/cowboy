import { requestPriorSendDecision } from "./priorSendDecision";

export interface PendingAuthoredSend {
  readonly id: string;
  readonly authored: boolean;
  readonly held: boolean;
  readonly sourceCmid?: string;
}

interface AuthoredSendDependencies {
  hydrate: (sessionId: string) => Promise<void>;
  pending: (sessionId: string) => readonly PendingAuthoredSend[];
  decide?: (sessionId: string, ids: readonly string[]) => Promise<void>;
}

/** Coordinates every authored send before its source or durable outbox changes.
 * The adapter owns persistence; the decision surface owns user interaction.
 */
export function createAuthoredSendGate(dependencies: AuthoredSendDependencies) {
  const decide = dependencies.decide ?? requestPriorSendDecision;
  return {
    hasPending(sessionId: string, ids: readonly string[]): boolean {
      return dependencies.pending(sessionId).some((message) =>
        ids.includes(message.id)
      );
    },
    async prepare(sessionId: string, sourceCmid?: string): Promise<void> {
      await dependencies.hydrate(sessionId);
      for (;;) {
        const older = dependencies.pending(sessionId).filter((message) =>
          message.authored && message.held &&
          (sourceCmid === undefined ||
            (message.id !== sourceCmid && message.sourceCmid !== sourceCmid))
        ).map((message) => message.id);
        if (older.length === 0) return;
        await decide(sessionId, older);
        // A decision or late receipt can change the outbox while we wait.
        // Recheck before allowing the caller to consume its source.
      }
    },
  };
}
