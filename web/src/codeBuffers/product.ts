/** Core integration port. The real product identity/dataset owner supplies
 * invalidation; its public descriptor is NOT a native grant or an open effect.
 * No import, render or ready() call opens a buffer or an IndexedDB connection.
 */
import { productSyncDatabase } from "../productSyncDatabase.ts";
import { createOwnedCodeBuffers } from "./owner.ts";
import { BufferClientError } from "./protocol.ts";
import { observePromise, type TransportOptions } from "./transport.ts";

type Source = Pick<typeof productSyncDatabase, "ready" | "signal">;

export function createProductCodeBuffers(
  source: Source,
  options: Omit<TransportOptions, "context"> = {},
) {
  const context = source.signal;
  const discover = source.ready.bind(source);
  const buffers = createOwnedCodeBuffers({ ...options, context });
  const check = () => {
    if (context.aborted) throw new BufferClientError("context_lost");
  };
  return Object.freeze({
    /** Local-only projection; viewing it never discovers or opens resources. */
    cleanup: buffers.cleanup,
    synchronizations: buffers.synchronizations,
    async ready(observer?: AbortSignal): Promise<typeof buffers> {
      check();
      if (observer?.aborted) throw new BufferClientError("cancelled");
      // One view may stop waiting, but cannot cancel shared Service discovery.
      // Ending the core context rejects even if discovery ignores cancellation.
      const bound = async () => {
        try {
          await observePromise(
            Promise.resolve().then(() => {
              check();
              return discover();
            }),
            context,
          );
          check();
          return buffers;
        } catch {
          check();
          throw new BufferClientError("transport");
        }
      };
      return observePromise(bound(), observer);
    },
  });
}

// Shared across Review mounts; construction has no transport or storage effect.
// Review selects this only from the Service's owned-mode observation, never as
// a retry/fallback after a legacy request or through serialized resource IDs.
export const productCodeBuffers = createProductCodeBuffers(productSyncDatabase);
