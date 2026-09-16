/** Public fake identities and deferred HTTP only. Not a product entrypoint. */
import { createOwnedCodeBuffers } from "./owner.ts";
import type { BufferFetch } from "./transport.ts";
import type { BufferState, ReadKind } from "./protocol.ts";

export const ID = "0123456789abcdef0123456789abcdef-0000000000000001";
export const OTHER = "0123456789abcdef0123456789abcdef-0000000000000002";
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
export function wire(state: BufferState, resourceId = ID, pending = false) {
  return { apiVersion: 1, resourceId, state, pending };
}
export function readWire(kind: ReadKind, resourceId = ID) {
  return {
    apiVersion: 1,
    resourceId,
    openedVersion: [],
    result: kind === "language"
      ? {
        kind,
        diagnosticsState: "unobserved",
        diagnostics: [],
        inlayHints: [],
        semanticTokens: [],
      }
      : { kind, symbols: [] },
  };
}
export function fixture(timeoutMs = 65_000) {
  const context = new AbortController();
  const calls: {
    url: string;
    init: RequestInit;
    result: ReturnType<typeof deferred<Response>>;
  }[] = [];
  const fetch: BufferFetch = (url, init) => {
    const result = deferred<Response>();
    calls.push({ url, init, result });
    return result.promise;
  };
  const registry = createOwnedCodeBuffers({
    context: context.signal,
    fetch,
    timeoutMs,
  });
  const owner = registry.reserve({
    sessionId: "session/a",
    path: "src/main.rs",
  });
  const reply = (index: number, value: unknown, status = 200) => {
    const call = calls[index];
    if (!call) throw new Error(`missing fixture request ${index}`);
    call.result.resolve(Response.json(value, { status }));
  };
  const advance = async (count: number) => {
    for (let n = 0; n < 100 && calls.length < count; n++) {
      await Promise.resolve();
    }
    if (calls.length !== count) {
      throw new Error(`expected ${count} calls, observed ${calls.length}`);
    }
  };
  return { context, registry, owner, calls, reply, advance };
}
export async function opened(timeoutMs = 65_000) {
  const value = fixture(timeoutMs);
  const prepared = value.owner.prepare();
  value.reply(0, wire("prepared"));
  await prepared;
  const active = value.owner.open();
  value.reply(1, wire("open"));
  await active;
  return value;
}
