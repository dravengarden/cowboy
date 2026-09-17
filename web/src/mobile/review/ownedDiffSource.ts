import type { BufferTarget } from "../../codeBuffers/owner.ts";
import { productCodeBuffers } from "../../codeBuffers/product.ts";
import { BufferClientError } from "../../codeBuffers/protocol.ts";
import { observePromise } from "../../codeBuffers/transport.ts";
import { productSyncDatabase } from "../../productSyncDatabase.ts";
import {
  type CodeDocument,
  fetchCodeFile,
  fetchCodeFilePage,
} from "./codeApi.ts";
import { reviewDisplayText } from "./reviewDisplayText.ts";

export interface ReviewDiffSource {
  readonly signal: AbortSignal;
  ready(observer: AbortSignal): Promise<unknown>;
  read(
    target: Readonly<BufferTarget>,
    cursor: string | undefined,
    observer: AbortSignal,
  ): Promise<CodeDocument & { size: number }>;
}

/** The existing core product lifetime; neither a cookie nor a path is a grant. */
export const productReviewDiffSource: ReviewDiffSource = Object.freeze({
  signal: productSyncDatabase.signal,
  ready: (observer: AbortSignal) => productCodeBuffers.ready(observer),
  read: (
    target: Readonly<BufferTarget>,
    cursor: string | undefined,
    observer: AbortSignal,
  ) =>
    cursor === undefined
      ? fetchCodeFile(target.sessionId, target.path, observer)
      : fetchCodeFilePage(target.sessionId, target.path, cursor, observer),
});

/** Bounded, original-context file observation. No retry, cursor restart,
 * changed-revision merge or native open/reload. Native equality is checked
 * independently on EVERY later conditional Code read, not inferred from stat.
 */
export async function readReviewDiffSource(
  port: ReviewDiffSource,
  input: BufferTarget,
  observer: AbortSignal,
): Promise<string> {
  const target = Object.freeze({ ...input });
  const read = port.read.bind(port), ready = port.ready.bind(port);
  const signal = AbortSignal.any([
    port.signal,
    observer,
    AbortSignal.timeout(65_000),
  ]);
  const check = () => {
    if (signal.aborted) throw new BufferClientError("cancelled");
  };
  check();
  await observePromise(ready(signal), signal);
  let cursor: string | undefined;
  let revision: string | undefined, size: number | undefined;
  let bytes = 0;
  const chunks: string[] = [], cursors = new Set<string>();
  for (let page = 0; page < 32; page++) {
    check();
    const value = await observePromise(read(target, cursor, signal), signal);
    check();
    if (
      !value || value.apiVersion !== 1 || value.path !== target.path ||
      typeof value.text !== "string" || value.text.length > 4 * 1024 * 1024 ||
      /[\uD800-\uDFFF]/u.test(value.text) ||
      typeof value.revision !== "string" || !value.revision ||
      value.revision.length > 512 || typeof value.truncated !== "boolean" ||
      (value.limited !== undefined && value.limited !== false) ||
      !Number.isSafeInteger(value.size) || value.size < 0 ||
      value.size > 4 * 1024 * 1024 ||
      (revision !== undefined && value.revision !== revision) ||
      (size !== undefined && value.size !== size)
    ) throw new BufferClientError("protocol");
    revision = value.revision;
    size = value.size;
    bytes += new TextEncoder().encode(value.text).length;
    if (bytes > size) throw new BufferClientError("protocol");
    chunks.push(value.text);
    // The real Rust response serializes an absent cursor as null.
    if (value.nextCursor == null) {
      if (value.truncated || bytes !== size) {
        throw new BufferClientError("protocol");
      }
      return reviewDisplayText(chunks.join(""));
    }
    cursor = value.nextCursor;
    if (
      typeof cursor !== "string" || !cursor || cursor.length > 512 ||
      cursors.has(cursor) || !value.text || !value.truncated || bytes >= size
    ) throw new BufferClientError("protocol");
    cursors.add(cursor);
  }
  throw new BufferClientError("capacity");
}
