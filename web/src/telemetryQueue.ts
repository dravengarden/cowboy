/** Pure bounded queue: retries retain their exact bytes, identity and context. */
export type TelemetryContext = {
  session_id?: string;
  machine_id?: string;
  trace_id?: string;
};
type Kind = "logs" | "metrics" | "incidents";
interface Entry {
  kind: Kind;
  value: Record<string, unknown>;
  context: TelemetryContext;
  key: string;
  bytes: number;
}
export interface QueuedBatch {
  readonly body: string;
  readonly items: number;
  readonly bytes: number;
  readonly created: number;
}

const encoder = new TextEncoder();
const MAX_ITEMS = 200;
const MAX_BYTES = 256 * 1024;
export const MAX_TELEMETRY_BODY_BYTES = 24 * 1024;

export function utf8Prefix(value: string, bytes: number): string {
  // Avoid replacement characters from a partially sliced UTF-8 sequence.
  let result = "";
  let length = 0;
  for (const character of value) {
    length += encoder.encode(character).length;
    if (length > bytes) break;
    result += character;
  }
  return result;
}

export function cleanTelemetryMessage(value: unknown): string {
  let text: string;
  try {
    text = value instanceof Error
      ? value.message
      : String(value ?? "Unknown error");
  } catch {
    text = "Unserializable error";
  }
  return utf8Prefix(
    utf8Prefix(text, 16 * 1024)
      .replace(/\b(?:set-cookie|cookie)\s*[:=][^\r\n]*/gim, "cookie=[redacted]")
      .replace(
        /\b(authorization|cookie|password|secret|api[_-]?key|(?:access[_-]?|refresh[_-]?|id[_-]?)token|client[_-]?secret)\b["']?\s*[:=]\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|(?:Bearer|Basic)\s+[^\s,;]+|[^\s,;]+)/gi,
        "$1=[redacted]",
      )
      .replace(/\b(Bearer|Basic)\s+[a-z\d._~+/-]+=*/gi, "$1 [redacted]")
      .replace(/https?:\/\/[^\s<>"']+/gi, (raw) => {
        try {
          const url = new URL(raw);
          url.username = "";
          url.password = "";
          url.search = "";
          url.hash = "";
          return url.toString();
        } catch {
          return "[redacted-url]";
        }
      }),
    4096,
  );
}

export function cleanTelemetryAttributes(
  values: Record<string, unknown>,
): Record<string, string | number | boolean | null> {
  const result: Record<string, string | number | boolean | null> = {};
  for (const [key, value] of Object.entries(values)) {
    if (Object.keys(result).length === 16) break;
    if (
      !/^[a-zA-Z0-9_.:-]{1,64}$/.test(key) ||
      /token|secret|password|authorization|cookie|clipboard|prompt/i.test(key)
    ) continue;
    if (typeof value === "string") {
      result[key] = utf8Prefix(cleanTelemetryMessage(value), 512);
    } else if (
      value === null || typeof value === "boolean" ||
      (typeof value === "number" && Number.isFinite(value))
    ) result[key] = value;
  }
  return result;
}

export function retryTelemetryStatus(status: number): boolean {
  return status === 408 || status === 429 || status >= 500;
}

export class TelemetryQueue {
  private entries: Entry[] = [];
  private current: QueuedBatch | null = null;
  private attempts = 0;
  private retryAt = 0;
  dropped = 0;

  get size(): number {
    return this.entries.length + (this.current?.items ?? 0);
  }
  get bytes(): number {
    return this.entries.reduce(
      (sum, item) => sum + item.bytes,
      this.current?.bytes ?? 0,
    );
  }

  capture(
    kind: Kind,
    value: Record<string, unknown>,
    context: TelemetryContext,
  ): void {
    try {
      const key = JSON.stringify(context);
      const serialized = JSON.stringify(value);
      const bytes = encoder.encode(serialized + key).length;
      if (bytes > MAX_TELEMETRY_BODY_BYTES - 2048) {
        this.dropped++;
        return;
      }
      this.entries.push({
        kind,
        value: JSON.parse(serialized),
        context: JSON.parse(key),
        key,
        bytes,
      });
      // Reserve the bounded client/batch envelope before entries become an
      // in-flight body; taking a batch must not itself exceed the byte budget.
      while (this.size > MAX_ITEMS || this.bytes > MAX_BYTES - 2048) {
        let index = this.entries.findIndex((entry) =>
          entry.kind === "logs" && entry.value.level === "debug"
        );
        if (index < 0) {
          index = this.entries.findIndex((entry) => entry.kind === "metrics");
        }
        if (index < 0) {
          index = this.entries.findIndex((entry) => entry.kind === "logs");
        }
        this.entries.splice(Math.max(0, index), 1); // Incident-only floods are bounded too.
        this.dropped++;
      }
    } catch {
      this.dropped++; // Diagnostic collection must never throw into product code.
    }
  }

  take(
    client: Record<string, unknown>,
    id: () => string,
    now = Date.now(),
  ): QueuedBatch | null {
    if (this.current && now - this.current.created > 5 * 60_000) {
      this.dropped += this.current.items;
      this.current = null;
    }
    if (now < this.retryAt) return null;
    if (this.current) return this.current;
    const first = this.entries[0];
    if (!first) return null;
    const batch = {
      batch_id: id(),
      client,
      context: first.context,
      logs: [] as unknown[],
      metrics: [] as unknown[],
      incidents: [] as unknown[],
    };
    let body = JSON.stringify(batch);
    let items = 0;
    while (this.entries[0]?.key === first.key && items < MAX_ITEMS) {
      const entry = this.entries[0]!;
      batch[entry.kind].push(entry.value);
      const next = JSON.stringify(batch);
      if (encoder.encode(next).length > MAX_TELEMETRY_BODY_BYTES) {
        batch[entry.kind].pop();
        if (items === 0) {
          this.entries.shift();
          this.dropped++;
        }
        break;
      }
      this.entries.shift();
      body = next;
      items++;
    }
    if (items === 0) return null;
    this.attempts = 0;
    this.current = {
      body,
      items,
      bytes: encoder.encode(body).length,
      created: now,
    };
    return this.current;
  }

  settle(batch: QueuedBatch, retry: boolean, now = Date.now()): void {
    if (this.current !== batch) return; // Sign-out cleared an older in-flight batch.
    if (retry && ++this.attempts < 5 && now - batch.created < 5 * 60_000) {
      this.retryAt = now + Math.min(30_000, 1000 * 2 ** (this.attempts - 1));
      return;
    }
    if (retry) this.dropped += batch.items;
    this.current = null;
    this.retryAt = 0;
  }

  clear(): void {
    this.entries = [];
    this.current = null;
    this.retryAt = 0;
    this.attempts = 0;
  }
}
