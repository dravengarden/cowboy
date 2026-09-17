/** Complete text from one original native owner, never a path/disk fallback.
 * Pages and opaque revision markers stay inside this one bounded read job.
 */
import {
  captureContent,
  type CapturedContent,
  capturedIdentity,
  type ContentIdentity,
} from "./content.ts";
import { decodeReadEnvelope } from "./observations.ts";
import {
  BufferClientError,
  integer,
  record,
  requireValue,
  type ResourceId,
  text,
} from "./protocol.ts";

export type TextRead =
  | { readonly kind: "complete"; readonly content: CapturedContent }
  | { readonly kind: "mismatch" | "stale" };

export function textIdentity(value: ContentIdentity): ContentIdentity {
  const row = record(value, ["sha256", "utf8Bytes"]);
  const sha256 = snapshotId(row.sha256);
  const utf8Bytes = integer(row.utf8Bytes, 0, 4 * 1024 * 1024);
  return Object.freeze({ sha256, utf8Bytes });
}

type Page = { readonly kind: "start" } | {
  readonly kind: "continue";
  readonly snapshot: string;
  readonly offset: number;
};
interface Request {
  readonly kind: "text";
  readonly content: ContentIdentity;
  readonly page: Page;
}
function snapshotId(value: unknown): string {
  requireValue(typeof value === "string" && /^[0-9a-f]{64}$/.test(value));
  return value;
}

export async function readCompleteText(
  id: ResourceId,
  content: ContentIdentity,
  request: (body: Request) => Promise<{ value: unknown; status: number }>,
  check: () => void,
): Promise<TextRead> {
  const until = performance.now() + 60_000;
  const current = () => {
    check();
    if (performance.now() >= until) throw new BufferClientError("transport");
  };
  let page: Page = { kind: "start" };
  const parts: string[] = [];
  for (let count = 0; count < 65; count++) {
    current();
    const reply = await request({ kind: "text", content, page });
    current(); // cancelled/closing/ended authority cannot start another page
    requireValue(reply.status === 200);
    const envelope = decodeReadEnvelope(reply.value, id);
    const outer = record(envelope.result, ["kind", "content", "result"]);
    requireValue(outer.kind === "text");
    const identity = record(outer.content, ["sha256", "utf8Bytes"]);
    requireValue(
      identity.sha256 === content.sha256 &&
        identity.utf8Bytes === content.utf8Bytes,
    );
    requireValue(outer.result && typeof outer.result === "object");
    const kind = (outer.result as { kind?: unknown }).kind;
    if (kind === "mismatch" || kind === "stale") {
      record(outer.result, ["kind"]);
      requireValue(kind !== "stale" || page.kind === "continue");
      return Object.freeze({ kind });
    }
    const result = record(outer.result, [
      "kind",
      "snapshot",
      "offset",
      "text",
      "nextOffset",
    ]);
    requireValue(result.kind === "page");
    const snapshot = snapshotId(result.snapshot);
    const offset: number = page.kind === "start" ? 0 : page.offset;
    requireValue(
      result.offset === offset &&
        (page.kind === "start" || page.snapshot === snapshot),
    );
    const part = text(result.text, 65_536);
    requireValue(!part.includes("\r") && !/[\uD800-\uDFFF]/u.test(part));
    const bytes = new TextEncoder().encode(part).length;
    const end: number = offset + bytes;
    requireValue(end <= content.utf8Bytes);
    parts.push(part);
    if (end === content.utf8Bytes) {
      requireValue(result.nextOffset === null);
      const captured = await captureContent(parts.join(""));
      current(); // actual WebCrypto is asynchronous too
      const exact = capturedIdentity(captured);
      requireValue(
        exact.sha256 === content.sha256 &&
          exact.utf8Bytes === content.utf8Bytes,
      );
      return Object.freeze({ kind: "complete", content: captured });
    }
    requireValue(result.nextOffset === end && bytes >= 65_533);
    page = { kind: "continue", snapshot, offset: end };
  }
  throw new BufferClientError("protocol");
}
