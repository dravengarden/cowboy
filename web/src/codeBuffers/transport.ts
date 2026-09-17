import { BufferClientError, requireValue } from "./protocol.ts";

export type BufferFetch = (url: string, init: RequestInit) => Promise<Response>;
export interface TransportOptions {
  /** Core Service/principal lifetime; NOT the dismissible Review view. */
  readonly context: AbortSignal;
  readonly fetch?: BufferFetch;
  readonly timeoutMs?: number;
}

export function createTransport(options: TransportOptions) {
  const context = options.context;
  const send = options.fetch ?? globalThis.fetch.bind(globalThis);
  const timeout = options.timeoutMs ?? 65_000;
  requireValue(Number.isInteger(timeout) && timeout > 0 && timeout <= 65_000);
  const check = () => {
    if (context.aborted) throw new BufferClientError("context_lost");
  };
  return {
    check,
    async request(
      path: string,
      method: "POST" | "PUT" | "GET" | "DELETE",
      body: unknown,
      limit: number,
      surface: "buffers" | "buffer-synchronizations" = "buffers",
    ) {
      check();
      requireValue(
        surface === "buffers" || surface === "buffer-synchronizations",
      );
      const controller = new AbortController();
      const abort = () => controller.abort();
      const interrupted = new Promise<never>((_resolve, reject) => {
        controller.signal.addEventListener(
          "abort",
          () => reject(new BufferClientError("transport")),
          { once: true },
        );
      });
      context.addEventListener("abort", abort, { once: true });
      const timer = setTimeout(abort, timeout);
      let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
      try {
        const sent = send(`/api/code/${surface}${path}`, {
          method,
          credentials: "same-origin",
          mode: "same-origin",
          cache: "no-store",
          redirect: "error",
          signal: controller.signal,
          ...(method === "GET" ? {} : {
            headers: { "content-type": "application/json" },
            body: JSON.stringify(body),
          }),
        }).then((response) => {
          if (controller.signal.aborted) {
            void response.body?.cancel().catch(() => undefined);
            throw new BufferClientError("transport");
          }
          return response;
        });
        const response = await Promise.race([sent, interrupted]);
        reader = response.body?.getReader();
        check();
        if (response.status !== 200 && response.status !== 202) {
          throw new BufferClientError("http", response.status);
        }
        requireValue(
          response.headers.get("content-type")?.split(";")[0]?.trim()
                .toLowerCase() === "application/json" && reader,
        );
        const chunks: Uint8Array[] = [];
        let size = 0;
        for (;;) {
          const { done, value } = await Promise.race([
            reader.read(),
            interrupted,
          ]);
          check();
          if (controller.signal.aborted) {
            throw new BufferClientError("transport");
          }
          if (done) break;
          size += value.byteLength;
          requireValue(size <= limit && chunks.length < 65_536);
          chunks.push(value);
        }
        const bytes = new Uint8Array(size);
        let offset = 0;
        for (const chunk of chunks) {
          bytes.set(chunk, offset);
          offset += chunk.length;
        }
        let value: unknown;
        try {
          value = JSON.parse(
            new TextDecoder("utf-8", { fatal: true }).decode(bytes),
          );
        } catch {
          throw new BufferClientError("protocol");
        }
        check();
        return { value, status: response.status };
      } catch (error) {
        check();
        throw error instanceof BufferClientError
          ? error
          : new BufferClientError("transport");
      } finally {
        clearTimeout(timer);
        context.removeEventListener("abort", abort);
        // Cancellation must not retain the owner on an untrusted stream's
        // asynchronous cancel promise. No response body is a diagnostic log.
        void reader?.cancel().catch(() => undefined);
        reader?.releaseLock();
      }
    },
  };
}

/** Detaching one observer never cancels an admitted owner continuation. */
export function observePromise<T>(
  promise: Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  if (!signal) return promise;
  return new Promise((resolve, reject) => {
    const abort = () => reject(new BufferClientError("cancelled"));
    if (signal.aborted) abort();
    else signal.addEventListener("abort", abort, { once: true });
    promise.then(
      (value) => signal.aborted ? abort() : resolve(value),
      reject,
    ).finally(() => signal.removeEventListener("abort", abort));
  });
}
