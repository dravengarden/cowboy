import {
  ProtobufLogsSerializer,
  ProtobufMetricsSerializer,
  ProtobufTraceSerializer,
} from "@opentelemetry/otlp-transformer";
import { newUuid } from "./uuid.ts";

export type OtlpSignal = "logs" | "metrics" | "traces";
export const OTLP_MAX_BODY = 24 * 1024;
const MAX_BYTES = 256 * 1024;
const MAX_ITEMS = 200;
const MAX_AGE = 5 * 60_000;
interface Batch {
  signal: OtlpSignal;
  id: string;
  body: Uint8Array<ArrayBuffer>;
  items: number;
  born: number;
  next: number;
  attempts: number;
}

/** One owner of network retries. SDK exporters acknowledge bounded admission,
 * not remote persistence. Retried bodies and IDs are immutable. */
export class OtlpTransport {
  private queue: Batch[] = [];
  private active: AbortController | undefined;
  private inFlight: Batch | undefined;
  private flushing: Promise<void> | undefined;
  private stopped = false;
  readonly health = { droppedBatches: 0, rejectedItems: 0, failedBatches: 0 };

  constructor(
    private readonly request: typeof fetch = (...args) =>
      globalThis.fetch(...args),
    private readonly now: () => number = Date.now,
    private readonly id: () => string = newUuid,
  ) {}

  get pendingBytes(): number {
    return this.queue.reduce((n, b) => n + b.body.byteLength, 0);
  }
  get pendingItems(): number {
    return this.queue.reduce((n, b) => n + b.items, 0);
  }

  enqueue(
    signal: OtlpSignal,
    body: Uint8Array | undefined,
    items: number,
  ): boolean {
    if (this.stopped || items === 0) return false;
    this.prune();
    if (
      !body || body.byteLength > OTLP_MAX_BODY ||
      this.pendingBytes + body.byteLength > MAX_BYTES ||
      this.pendingItems + items > MAX_ITEMS
    ) {
      this.health.droppedBatches++;
      return false;
    }
    this.queue.push({
      signal,
      body: new Uint8Array(body),
      items,
      id: this.id(),
      born: this.now(),
      next: 0,
      attempts: 0,
    });
    return true;
  }

  private prune(): void {
    this.queue = this.queue.filter((b) => {
      if (b === this.inFlight) return true;
      if (this.now() - b.born < MAX_AGE && b.attempts < 5) return true;
      this.health.droppedBatches++;
      return false;
    });
  }

  private url(batch: Batch): string {
    return `/api/telemetry/v1/${batch.signal}?batch_id=${
      encodeURIComponent(batch.id)
    }`;
  }

  flush(): Promise<void> {
    return this.flushing ??= this.run().finally(() => {
      this.flushing = undefined;
    });
  }

  private async run(): Promise<void> {
    const deadline = this.now() + 8000;
    while (!this.stopped && this.now() < deadline) {
      this.prune();
      const batch = this.queue.find((b) => b.next <= this.now());
      if (!batch) return;
      const controller = new AbortController();
      this.active = controller;
      this.inFlight = batch;
      const timeout = setTimeout(
        () => controller.abort(),
        Math.max(1, deadline - this.now()),
      );
      let retry = false;
      let responded = false;
      try {
        const response = await this.request(this.url(batch), {
          method: "POST",
          credentials: "same-origin",
          redirect: "error",
          headers: { "content-type": "application/x-protobuf" },
          body: batch.body,
          signal: controller.signal,
        });
        responded = true;
        if (response.status === 200) {
          // A 200 (even partial/warning-only) must never be retried. Do not
          // retain or log backend error_message text; it may contain secrets.
          const body = await boundedResponse(response);
          const serializer = batch.signal === "logs"
            ? ProtobufLogsSerializer
            : batch.signal === "metrics"
            ? ProtobufMetricsSerializer
            : ProtobufTraceSerializer;
          const partial = serializer.deserializeResponse(body).partialSuccess;
          const count = Number(
            partial &&
              ("rejectedLogRecords" in partial
                ? partial.rejectedLogRecords
                : "rejectedDataPoints" in partial
                ? partial.rejectedDataPoints
                : "rejectedSpans" in partial
                ? partial.rejectedSpans
                : 0),
          ) || 0;
          if (
            !Number.isSafeInteger(count) || count < 0 || count > batch.items
          ) {
            throw new Error("Invalid OTLP receipt");
          }
          this.health.rejectedItems += count;
        } else {
          retry = [429, 502, 503, 504].includes(response.status);
          if (!retry) this.health.failedBatches++;
          await response.body?.cancel();
        }
      } catch {
        // A response parsing failure follows a successful receipt, and must
        // not duplicate that delivery. Only pre-response network errors retry.
        retry = !responded && !this.stopped;
        this.health.failedBatches++;
      } finally {
        clearTimeout(timeout);
        this.active = undefined;
        this.inFlight = undefined;
      }
      if (this.stopped) return;
      if (retry) {
        batch.attempts++;
        batch.next = this.now() + Math.min(30_000, 1000 * 2 ** batch.attempts);
      } else {
        this.queue = this.queue.filter((b) => b !== batch);
      }
    }
  }

  beacon(
    send: (url: string, body: Blob) => boolean = (url, body) =>
      navigator.sendBeacon(url, body),
  ): void {
    if (this.stopped || this.active) return;
    this.prune();
    let sent = 0;
    for (const batch of [...this.queue]) {
      if (batch.next > this.now() || sent + batch.body.byteLength > 48 * 1024) {
        continue;
      }
      try {
        if (
          send(
            this.url(batch),
            new Blob([batch.body], { type: "application/x-protobuf" }),
          )
        ) {
          this.queue = this.queue.filter((b) => b !== batch);
          sent += batch.body.byteLength;
        }
      } catch { /* Page-exit delivery is best effort. */ }
    }
  }

  stop(): void {
    this.stopped = true;
    this.active?.abort();
    this.queue = [];
  }
}

async function boundedResponse(response: Response): Promise<Uint8Array> {
  if (
    response.headers.get("content-type")?.split(";")[0] !==
      "application/x-protobuf"
  ) throw new Error("Invalid OTLP content type");
  const reader = response.body?.getReader();
  const parts: Uint8Array[] = [];
  let size = 0;
  if (reader) {
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 64 * 1024) throw new Error("OTLP response too large");
        parts.push(value);
      }
    } finally {
      await reader.cancel();
    }
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.byteLength;
  }
  return bytes;
}
