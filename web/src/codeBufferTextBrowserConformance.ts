/** Real browser WebCrypto, Response streams and cancellation; fixture HTTP only.
 * This is not the eventual Review destination consumer or physical-device gate.
 */
import { captureContent, capturedIdentity } from "./codeBuffers/content.ts";
import { ID, opened, wire } from "./codeBuffers/fixture.ts";
import { BufferClientError } from "./codeBuffers/protocol.ts";

function check(value: unknown): asserts value {
  if (!value) throw new Error("native text browser fixture failed");
}
async function calls(f: Awaited<ReturnType<typeof opened>>, count: number) {
  for (let n = 0; n < 300 && f.calls.length < count; n++) {
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  check(f.calls.length === count);
}
async function refusal(task: Promise<unknown>, kind: string) {
  try {
    await task;
  } catch (error) {
    check(error instanceof BufferClientError && error.kind === kind);
    return;
  }
  throw new Error("native text unexpectedly succeeded");
}

export async function runTextBrowserConformance(): Promise<string[]> {
  const first = "a".repeat(65_535), last = "🙂\0\n";
  const content = capturedIdentity(await captureContent(first + last));
  const reply = (offset: number, text: string, nextOffset: number | null) => ({
    apiVersion: 1,
    resourceId: ID,
    openedVersion: [],
    result: {
      kind: "text",
      content,
      result: {
        kind: "page",
        snapshot: "a".repeat(64),
        offset,
        text,
        nextOffset,
      },
    },
  });
  const tests: string[] = [];
  const view = document.createElement("pre");
  document.body.append(view);
  const f = await opened();
  try {
    const read = f.owner.readText(content).then((result) => {
      check(result.kind === "complete");
      view.textContent = result.content.text;
      check(capturedIdentity(result.content).sha256 === content.sha256);
    });
    f.reply(2, reply(0, first, 65_535));
    await calls(f, 4);
    check(view.textContent === "" && f.owner.view().busy === "read");
    f.reply(3, reply(65_535, last, null));
    await read;
    check(view.textContent === first + last);
    tests.push(
      "native pages expose complete Unicode text only after browser SHA-256 verification",
    );
    const bad = f.owner.readText(content);
    f.reply(4, reply(0, first, 65_535));
    await calls(f, 6);
    f.reply(5, reply(65_535, "🙃\0\n", null));
    await refusal(bad, "protocol");
    check(f.calls.length === 6 && view.textContent === first + last);
    tests.push(
      "equal-length corrupted final page cannot become a content capture or change displayed text",
    );
  } finally {
    f.context.abort();
    view.remove();
  }
  const g = await opened();
  try {
    const observer = new AbortController();
    const read = g.owner.readText(content, observer.signal);
    const rejected = refusal(read, "cancelled");
    observer.abort();
    await rejected;
    const close = g.owner.close();
    check(g.owner.view().busy === "read" && !g.calls[2]!.init.signal?.aborted);
    g.reply(2, reply(0, first, 65_535));
    await calls(g, 4);
    check(g.calls[3]!.init.method === "GET");
    g.reply(3, wire("open"));
    await calls(g, 5);
    check(g.calls[4]!.init.method === "DELETE");
    g.reply(4, wire("released"));
    check((await close).kind === "released");
    tests.push(
      "cancelled paged text drains the admitted read and original cleanup without requesting another page",
    );
  } finally {
    g.context.abort();
  }
  return tests;
}
