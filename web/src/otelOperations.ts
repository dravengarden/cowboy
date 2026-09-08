import type { ClientSpan } from "./otel.ts";

interface Operation {
  session: string;
  started: number;
  span?: ClientSpan | undefined;
  acknowledged: boolean;
  output?: ClientSpan | undefined;
  dispatched?: number;
}

/** Correlate only live, unambiguous echoes, never hydrated transcript history.
 * A different client's user echo cancels first-output attribution for a turn. */
export class ClientOperations {
  private pending = new Map<string, Operation>();
  constructor(
    private readonly start: (
      name: "command" | "first_output",
      attrs: Record<string, string>,
      parent?: ClientSpan,
    ) => ClientSpan | undefined,
    private readonly duration: (
      name: "command" | "first_output",
      ms: number,
      attrs: Record<string, string>,
    ) => void,
    private readonly now: () => number = () => performance.now(),
  ) {}

  submit(session: string, cmid: string): string | undefined {
    this.prune();
    const existing = this.pending.get(cmid);
    if (existing) {
      return existing.session === session
        ? existing.span?.traceparent
        : undefined;
    }
    if (this.pending.size >= 32) return;
    const span = this.start("command", {
      operation: "submit",
      transport: "websocket",
    });
    this.pending.set(cmid, {
      session,
      started: this.now(),
      span,
      acknowledged: false,
    });
    return span?.traceparent;
  }

  acknowledge(session: string, ids: readonly string[]): void {
    this.prune();
    for (const id of ids) {
      const op = this.pending.get(id);
      if (!op || op.session !== session || op.acknowledged) continue;
      op.acknowledged = true;
      op.span?.end();
      this.duration("command", this.now() - op.started, {
        operation: "submit",
        transport: "websocket",
      });
    }
  }

  userEcho(session: string, cmid?: string): void {
    this.prune();
    for (const [id, op] of this.pending) {
      if (
        op.session === session && op.dispatched !== undefined && id !== cmid
      ) {
        op.output?.end("cancelled");
        this.pending.delete(id);
      }
    }
    if (!cmid) return;
    this.acknowledge(session, [cmid]);
    const op = this.pending.get(cmid);
    if (!op || op.session !== session || op.dispatched !== undefined) return;
    op.dispatched = this.now();
    op.output = this.start("first_output", {
      operation: "submit",
      transport: "websocket",
    }, op.span);
  }

  firstOutput(session: string): void {
    this.prune();
    for (const [id, op] of this.pending) {
      if (op.session !== session || op.dispatched === undefined) continue;
      op.output?.end();
      this.duration("first_output", this.now() - op.dispatched, {
        operation: "submit",
        transport: "websocket",
      });
      this.pending.delete(id);
    }
  }

  clear(): void {
    for (const op of this.pending.values()) {
      op.span?.end("cancelled");
      op.output?.end("cancelled");
    }
    this.pending.clear();
  }

  private prune(): void {
    for (const [id, op] of this.pending) {
      if (this.now() - op.started <= 5 * 60_000) continue;
      op.span?.end("timeout");
      op.output?.end("timeout");
      this.pending.delete(id);
    }
  }
}
