/** Exact displayed text, not a filesystem snapshot, native version or grant. */
import { decodeObservation, decodeReadEnvelope } from "./observations.ts";
import {
  integer,
  list,
  type Observation,
  type Point,
  record,
  requireValue,
  type ResourceId,
  type Results,
  text,
} from "./protocol.ts";

declare const captured: unique symbol;
export interface CapturedContent {
  readonly [captured]: true;
  /** Render this complete LF text; a page, diff hunk or disk ETag is not enough. */
  readonly text: string;
}
interface ContentIdentity {
  readonly sha256: string;
  readonly utf8Bytes: number;
}
const identities = new WeakMap<CapturedContent, ContentIdentity>();
const MAX_TEXT = 4 * 1024 * 1024;

export async function captureContent(value: string): Promise<CapturedContent> {
  text(value, MAX_TEXT);
  // Never silently convert CRLF, Unicode normalization, BOMs or lone surrogates.
  // The producer must explicitly supply the exact text used by its editor.
  requireValue(!value.includes("\r") && !/[\uD800-\uDFFF]/u.test(value));
  const bytes = new TextEncoder().encode(value);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  const identity = Object.freeze({
    sha256: [...digest].map((b) => b.toString(16).padStart(2, "0")).join(""),
    utf8Bytes: bytes.length,
  });
  const snapshot = Object.freeze({ text: value }) as CapturedContent;
  identities.set(snapshot, identity);
  return snapshot;
}

export interface ContentQueries {
  language: { readonly kind: "language" };
  symbols: { readonly kind: "symbols" };
  hover: { readonly kind: "hover"; readonly position: Point };
}
export type ContentKind = keyof ContentQueries;
interface HoverBlock {
  readonly text: string;
  readonly language: string | null;
  readonly markdown: boolean;
}
interface ContentResults {
  language: {
    readonly kind: "observed";
    readonly observation: Results["language"];
  };
  symbols: {
    readonly kind: "observed";
    readonly observation: Results["symbols"];
  };
  hover: { readonly kind: "hover"; readonly contents: readonly HoverBlock[] };
}
export type ContentObservation<K extends ContentKind> =
  & Omit<Observation<"language">, "result">
  & {
    readonly result: {
      readonly kind: "content";
      readonly content: ContentIdentity;
      readonly result: ContentResults[K] | { readonly kind: "mismatch" };
    };
  };

interface ContentRequest<K extends ContentKind> {
  readonly kind: "content";
  readonly content: ContentIdentity;
  readonly query: ContentQueries[K];
}

export function contentRequest<Q extends ContentQueries[ContentKind]>(
  snapshot: CapturedContent,
  query: Q,
): ContentRequest<Q["kind"]> {
  const content = identities.get(snapshot);
  requireValue(content);
  let capturedQuery: ContentQueries[ContentKind];
  if (query.kind === "hover") {
    record(query, ["kind", "position"]);
    const point = record(query.position, ["row", "column"]);
    const row = integer(point.row), column = integer(point.column);
    let start = 0;
    for (let n = 0; n < row; n++) {
      const end = snapshot.text.indexOf("\n", start);
      requireValue(end !== -1);
      start = end + 1;
    }
    const newline = snapshot.text.indexOf("\n", start);
    const end = newline === -1 ? snapshot.text.length : newline;
    requireValue(column <= end - start);
    const at = start + column;
    const before = snapshot.text.charCodeAt(at - 1),
      after = snapshot.text.charCodeAt(at);
    requireValue(
      !(before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 &&
        after <= 0xdfff),
    );
    capturedQuery = Object.freeze({
      kind: "hover",
      position: Object.freeze({ row, column }),
    });
  } else {
    record(query, ["kind"]);
    requireValue(query.kind === "language" || query.kind === "symbols");
    capturedQuery = Object.freeze({ kind: query.kind });
  }
  return Object.freeze({
    kind: "content" as const,
    content,
    query: capturedQuery,
  }) as ContentRequest<Q["kind"]>;
}

export function decodeContentObservation<K extends ContentKind>(
  value: unknown,
  id: ResourceId,
  request: ContentRequest<K>,
): ContentObservation<K> {
  const envelope = decodeReadEnvelope(value, id);
  const row = record(envelope.result, ["kind", "content", "result"]);
  requireValue(row.kind === "content");
  const content = record(row.content, ["sha256", "utf8Bytes"]);
  requireValue(
    content.sha256 === request.content.sha256 &&
      content.utf8Bytes === request.content.utf8Bytes,
  );
  requireValue(row.result && typeof row.result === "object");
  let result: ContentResults["hover"] | { readonly kind: "mismatch" } | {
    readonly kind: "observed";
    readonly observation: Results["language" | "symbols"];
  };
  if ((row.result as { kind?: unknown }).kind === "mismatch") {
    record(row.result, ["kind"]);
    result = Object.freeze({ kind: "mismatch" });
  } else if (request.query.kind === "hover") {
    const hover = record(row.result, ["kind", "contents"]);
    requireValue(hover.kind === "hover");
    result = Object.freeze({
      kind: "hover",
      contents: Object.freeze(
        list(hover.contents, 32).map((value) => {
          const block = record(value, ["text", "language", "markdown"]);
          requireValue(typeof block.markdown === "boolean");
          return Object.freeze({
            text: text(block.text),
            language: block.language === null ? null : text(block.language),
            markdown: block.markdown,
          });
        }),
      ),
    });
  } else {
    const observed = record(row.result, ["kind", "observation"]);
    requireValue(observed.kind === "observed");
    const observation = decodeObservation(
      { ...envelope, result: observed.observation },
      id,
      request.query.kind,
    ).result;
    result = Object.freeze({ kind: "observed", observation });
  }
  return Object.freeze({
    ...envelope,
    result: Object.freeze({
      kind: "content",
      content: request.content,
      result,
    }),
  }) as ContentObservation<K>;
}
