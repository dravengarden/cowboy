import { assertEquals, assertRejects } from "jsr:@std/assert";
import { deferred } from "../../codeBuffers/fixture.ts";
import { BufferClientError } from "../../codeBuffers/protocol.ts";
import {
  readReviewDiffSource,
  type ReviewDiffSource,
} from "./ownedDiffSource.ts";

type Page = Awaited<ReturnType<ReviewDiffSource["read"]>>;
const target = { sessionId: "session", path: "a.ts" };
const signal = () => new AbortController().signal;
function page(
  text: string,
  next?: string,
  size = new TextEncoder().encode(text).length,
): Page {
  return {
    apiVersion: 1,
    path: "a.ts",
    revision: "revision",
    size,
    text,
    truncated: !!next,
    ...(next ? { nextCursor: next } : {}),
  };
}
function source(pages: Page[]) {
  const calls: (string | undefined)[] = [];
  const port: ReviewDiffSource = {
    signal: signal(),
    ready: () => Promise.resolve(),
    read: (_target, cursor) => {
      calls.push(cursor);
      return Promise.resolve(pages[calls.length - 1]!);
    },
  };
  return { port, calls };
}

Deno.test("diff source assembles bounded original-revision pages before LF normalization", async () => {
  const text = "a\r\nb🙂\r\n";
  const size = new TextEncoder().encode(text).length;
  const f = source([
    page("a\r", "next", size),
    page("\nb🙂\r\n", undefined, size),
  ]);
  assertEquals(
    await readReviewDiffSource(f.port, target, signal()),
    "a\nb🙂\n",
  );
  assertEquals(f.calls, [undefined, "next"]);
});
Deno.test("diff source freezes target and port before discovery and does not acquire a buffer", async () => {
  const ready = deferred<void>();
  const input = { ...target }, seen: unknown[] = [];
  const port: ReviewDiffSource = {
    signal: signal(),
    ready: () => ready.promise,
    read: (target) => {
      seen.push(target);
      return Promise.resolve(page("done\n"));
    },
  };
  const pending = readReviewDiffSource(port, input, signal());
  input.path = "wrong";
  port.read = () => Promise.reject(new Error("replacement port"));
  assertEquals(seen, []);
  ready.resolve();
  assertEquals(await pending, "done\n");
  assertEquals(seen, [target]);
});
Deno.test("diff source refuses changed revision, path, size and repeated cursors without restart", async () => {
  for (
    const next of [
      { ...page("b", undefined, 2), revision: "changed" },
      { ...page("b", undefined, 2), path: "other.ts" },
      page("b", undefined, 3),
      page("b", "next", 3),
    ]
  ) {
    const f = source([page("a", "next", next.size), next]);
    // A changed size must differ from the first observation as well.
    if (next.size === 3 && !next.nextCursor) {
      f.port.read = (_target, cursor) => {
        f.calls.push(cursor);
        return Promise.resolve(
          f.calls.length === 1 ? page("a", "next", 2) : next,
        );
      };
    }
    await assertRejects(
      () => readReviewDiffSource(f.port, target, signal()),
      BufferClientError,
    );
    assertEquals(f.calls, [undefined, "next"]);
  }
});
Deno.test("diff source refuses truncation, limits, invalid UTF-8 length and malformed paging", async () => {
  for (
    const value of [
      { ...page("a"), truncated: true },
      { ...page("a"), limited: true },
      page("a", undefined, 4 * 1024 * 1024 + 1),
      page("a", undefined, -1),
      page("🙂", undefined, 2),
      page("\ud800", undefined, 3),
      { ...page("a"), revision: "" },
      { ...page("a"), apiVersion: 2 } as unknown as Page,
      { ...page("a"), nextCursor: "" },
      page("", "next", 2),
      { ...page("a", "next", 2), truncated: false },
    ]
  ) {
    const f = source([value]);
    await assertRejects(
      () => readReviewDiffSource(f.port, target, signal()),
      BufferClientError,
    );
    assertEquals(f.calls, [undefined]);
  }
});
Deno.test("diff source has a finite page budget even when cursors keep changing", async () => {
  const f = source(
    Array.from({ length: 33 }, (_, n) => page("a", `next-${n}`, 100)),
  );
  await assertRejects(
    () => readReviewDiffSource(f.port, target, signal()),
    BufferClientError,
  );
  assertEquals(f.calls.length, 32);
});
Deno.test("an abandoned diff detaches from a late file read and cannot fetch another page", async () => {
  const reply = deferred<Page>(), observer = new AbortController();
  const f = source([]);
  f.port.read = (_target, cursor) => {
    f.calls.push(cursor);
    return reply.promise;
  };
  const pending = readReviewDiffSource(f.port, target, observer.signal);
  await Promise.resolve();
  await Promise.resolve();
  observer.abort();
  await assertRejects(() => pending, BufferClientError);
  reply.resolve(page("a", "next", 2));
  await Promise.resolve();
  await Promise.resolve();
  assertEquals(f.calls, [undefined]);
});
Deno.test("core identity loss during discovery prevents file I/O with replacement authority", async () => {
  const context = new AbortController(), ready = deferred<void>();
  let reads = 0;
  const pending = readReviewDiffSource(
    {
      signal: context.signal,
      ready: () => ready.promise,
      read: () => {
        reads++;
        return Promise.resolve(page("a"));
      },
    },
    target,
    signal(),
  );
  context.abort();
  await assertRejects(() => pending, BufferClientError);
  ready.resolve();
  await Promise.resolve();
  assertEquals(reads, 0);
});
Deno.test("failed file observation is not retried and nullable terminal cursors are accepted", async () => {
  const f = source([]);
  f.port.read = (_target, cursor) => {
    f.calls.push(cursor);
    return Promise.reject(new Error("private"));
  };
  await assertRejects(() => readReviewDiffSource(f.port, target, signal()));
  assertEquals(f.calls, [undefined]);
  const g = source([{ ...page("ok"), nextCursor: null } as unknown as Page]);
  assertEquals(await readReviewDiffSource(g.port, target, signal()), "ok");
});
