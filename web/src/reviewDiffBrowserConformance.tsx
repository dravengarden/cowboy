/** Real diff hook/status/CodeMirror with deferred file/native HTTP only. */
import { StrictMode, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { EditorView } from "@codemirror/view";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import {
  deferred,
  fixture,
  ID,
  readWire,
  wire,
} from "./codeBuffers/fixture.ts";
import { captureContent, capturedIdentity } from "./codeBuffers/content.ts";
import { BufferClientError } from "./codeBuffers/protocol.ts";
import CodeViewer, {
  type CodeInspectCandidate,
} from "./mobile/review/CodeViewer.tsx";
import type { CodeDiffScope } from "./mobile/review/codeApi.ts";
import type { ReviewDiffSource } from "./mobile/review/ownedDiffSource.ts";
import { ReviewDiffCodeStatus } from "./mobile/review/ReviewDiffCodeStatus.tsx";
import { useOwnedReviewDiff } from "./mobile/review/useOwnedReviewDiff.ts";
import { useOwnedReviewBuffer } from "./mobile/review/useOwnedReviewBuffer.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Diff fixture timed out: ${label}`);
}
type Page = Awaited<ReturnType<ReviewDiffSource["read"]>>;
function page(
  text: string,
  cursor?: string,
  size = new TextEncoder().encode(text).length,
): Page {
  return {
    apiVersion: 1,
    path: "a.txt",
    revision: "original-file",
    text,
    size,
    truncated: !!cursor,
    ...(cursor ? { nextCursor: cursor } : {}),
  };
}

export async function runReviewDiffBrowserConformance(): Promise<string[]> {
  const tests: string[] = [], f = fixture();
  const count = () => f.calls.length;
  const fileCount = () => files.length;
  await f.owner.close();
  const files: {
    cursor: string | undefined;
    reply: ReturnType<typeof deferred<Page>>;
  }[] = [];
  const source = { ready: () => Promise.resolve(f.registry) };
  const fileSource: ReviewDiffSource = {
    signal: f.context.signal,
    ready: source.ready,
    read: (_target, cursor) => {
      const reply = deferred<Page>();
      files.push({ cursor, reply });
      return reply.promise;
    },
  };
  const container = document.createElement("div");
  container.style.width = "360px";
  document.body.append(container);
  const root = createRoot(container);
  type Props = { patch: string | undefined; scope: CodeDiffScope };
  let latest: {
    diff: ReturnType<typeof useOwnedReviewDiff>;
    intelligence: ReturnType<typeof useOwnedReviewBuffer>;
  } | undefined;
  function Consumer({ patch, scope }: Props) {
    const diff = useOwnedReviewDiff(
      "fixture",
      "a.txt",
      true,
      scope,
      patch,
      fileSource,
    );
    const intelligence = useOwnedReviewBuffer(
      "fixture",
      "a.txt",
      scope !== "staged",
      diff.projection?.source,
      source,
      diff.projection,
    );
    const [hover, setHover] = useState<{ identity: unknown; text: string }>();
    latest = { diff, intelligence };
    const inspect = (candidates: CodeInspectCandidate[]) => {
      const point = candidates[0]!;
      void intelligence.hover(
        point.row,
        point.column,
        new AbortController().signal,
      ).then((value) => {
        setHover({
          identity: intelligence.identity,
          text: value.contents[0]?.text ?? "empty",
        });
      }).catch(() => undefined);
    };
    return (
      <>
        <ReviewDiffCodeStatus diff={diff} intelligence={intelligence} />
        <output>
          {hover?.identity === intelligence.identity ? hover?.text : "none"}
        </output>
        <CodeViewer
          text={patch ?? ""}
          kind="diff"
          path="a.txt"
          softWrap
          fontSize={14}
          diagnostics={false}
          inlayHints={false}
          semanticHighlighting={false}
          onInspect={diff.projection && intelligence.identity
            ? inspect
            : undefined}
          mapDiffPoint={diff.point}
          scrollRestoreKey="fixture-diff"
          onScrollTopChange={() => undefined}
        />
      </>
    );
  }
  const render = (
    patch: string | undefined,
    scope: CodeDiffScope = "unstaged",
  ) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={createTheme()}>
            <SurfaceProvider>
              <Consumer patch={patch} scope={scope} />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  const status = () =>
    container.querySelector(
      "[data-review-diff-status], [data-review-code-status]",
    )?.getAttribute("data-review-diff-status") ??
      container.querySelector("[data-review-code-status]")?.getAttribute(
        "data-review-code-status",
      );
  const calls = async (count: number) => {
    await until(() => f.calls.length >= count, `native request ${count}`);
    check(f.calls.length === count, "unexpected extra native request");
  };
  const fileCalls = async (count: number) => {
    await until(() => files.length >= count, `file request ${count}`);
    check(files.length === count, "unexpected file retry");
  };
  const click = (label: string) => {
    const button = [...container.querySelectorAll("button")].find((value) =>
      value.textContent === label
    );
    check(button, `missing ${label}`);
    flushSync(() => button.click());
  };
  const tap = async (row: number, column: number) => {
    const editor = container.querySelector<HTMLElement>(".cm-editor");
    check(editor, "missing actual CodeMirror");
    const view = EditorView.findFromDOM(editor);
    check(view, "missing actual EditorView");
    await until(
      () =>
        view.coordsAtPos(view.state.doc.line(row + 1).from + column) !== null,
      "rendered line",
    );
    const coords = view.coordsAtPos(
      view.state.doc.line(row + 1).from + column,
    )!;
    flushSync(() =>
      view.contentDOM.dispatchEvent(
        new MouseEvent("click", {
          bubbles: true,
          clientX: coords.left + 1,
          clientY: (coords.top + coords.bottom) / 2,
        }),
      )
    );
  };
  const patch =
    "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@\n context\n-old\n+a🙂z\n+last\n";
  const raw = "context\r\na🙂z\r\nlast\r\n";
  const content = await captureContent("context\na🙂z\nlast\n");
  const language = (mismatch = false) => ({
    ...readWire("language"),
    result: {
      kind: "content",
      content: capturedIdentity(content),
      result: mismatch
        ? { kind: "mismatch" }
        : { kind: "observed", observation: readWire("language").result },
    },
  });
  const hover = (text: string) => ({
    ...readWire("language"),
    result: {
      kind: "content",
      content: capturedIdentity(content),
      result: {
        kind: "hover",
        contents: [{ text, language: null, markdown: false }],
      },
    },
  });
  try {
    render(patch, "staged");
    await until(() => status() === "historical", "staged refusal");
    check(
      !fileCount() && !count(),
      "staged diff accessed current/native file",
    );
    check(
      !container.querySelector('[role="alert"]'),
      "historical diff occupied a status row",
    );
    render(undefined);
    await calls(1);
    f.reply(0, wire("prepared"));
    await calls(2);
    f.reply(1, wire("open"));
    await until(() => status() === "incomplete", "partial diff");
    check(
      !fileCount() && count() === 2,
      "partial diff supplied positions",
    );
    tests.push(
      "staged history starts no current-file/native operation; incomplete working diff performs no positional read",
    );

    render(patch);
    await fileCalls(1);
    files[0]!.reply.resolve(
      page("context\r", "page-two", new TextEncoder().encode(raw).length),
    );
    await fileCalls(2);
    check(
      files[1]!.cursor === "page-two",
      "file paging changed cursor lineage",
    );
    files[1]!.reply.resolve(
      page(
        raw.slice("context\r".length),
        undefined,
        new TextEncoder().encode(raw).length,
      ),
    );
    await calls(3);
    check(
      JSON.parse(String(f.calls[2]!.init.body)).content.sha256 ===
        capturedIdentity(content).sha256,
      "hashed the patch instead of complete LF source",
    );
    f.reply(2, language());
    await until(() => status() === "ready", "matching source");
    check(
      !container.querySelector('[role="alert"]'),
      "ready diff occupied a status row",
    );
    check(
      latest?.diff.projection?.source === content.text &&
        container.querySelector(".cm-content")?.textContent?.includes("-old"),
      "diff rendering/source identity missing",
    );
    tests.push(
      "actual diff hook pages one complete current file, normalizes after assembly and hashes source rather than patch",
    );

    await tap(5, 2);
    await new Promise<void>((resolve) => setTimeout(resolve, 20));
    check(
      count() === 3,
      "deleted-line tap borrowed a nearby new symbol",
    );
    await tap(6, 1);
    await calls(4);
    const query = JSON.parse(String(f.calls[3]!.init.body)).query;
    check(
      query.kind === "hover" && query.position.row === 1,
      "CodeMirror sent diff display coordinates",
    );
    f.reply(3, hover("new-side hover"));
    await until(
      () => container.querySelector("output")?.textContent === "new-side hover",
      "actual mapped hover",
    );
    tests.push(
      "actual CodeMirror rejects deleted-line taps and maps new-side taps through the complete-content proof",
    );

    check(latest, "missing intelligence");
    const stale = latest.intelligence.hover(1, 0, new AbortController().signal)
      .then(
        () => "accepted",
        (error: unknown) =>
          error instanceof BufferClientError ? error.kind : "wrong",
      );
    await calls(5);
    const second = patch.replace("-old", "-older");
    render(second, "combined");
    check(
      container.querySelector("output")?.textContent === "none",
      "old hover survived equal-source patch replacement",
    );
    check(await stale === "cancelled", "old patch observer survived commit");
    await fileCalls(3);
    files[2]!.reply.resolve(page(content.text));
    await until(() => !!latest?.diff.projection, "replacement projection");
    check(count() === 5, "replacement overlapped the old borrow");
    f.reply(4, hover("stale private hover"));
    await calls(6);
    f.reply(5, language());
    await until(() => status() === "ready", "replacement conditional read");
    check(
      f.calls[5]!.url === `/api/code/buffers/${ID}/read` &&
        !container.textContent?.includes("stale private hover"),
      "replaced owner or painted stale result",
    );
    tests.push(
      "changing patch/scope with equal full source ends old hover before paint and drains the same original owner without reopen",
    );

    render(second.replace("-older", "-oldest"));
    await fileCalls(4);
    files[3]!.reply.resolve(page(content.text.replace("last", "different")));
    await until(() => status() === "mismatch", "stale patch refusal");
    check(count() === 6, "stale patch queried native positions");
    check(
      container.querySelectorAll('[role="alert"]').length === 1,
      "mismatch must use one attention row",
    );
    click("Check file");
    await fileCalls(5);
    files[4]!.reply.resolve(page(content.text));
    await calls(7);
    f.reply(6, language(true));
    await until(
      () =>
        status() === "mismatch" &&
        !!container.querySelector('[title*="Open full source"]'),
      "native mismatch",
    );
    check(
      ![...container.querySelectorAll("button")].some((button) =>
        button.textContent === "Reload…" || button.textContent === "Confirm"
      ),
      "diff offered a native refresh",
    );
    check(
      container.querySelectorAll('[role="alert"]').length === 1,
      "native mismatch added a second status row",
    );
    click("Check");
    await calls(8);
    f.reply(7, language());
    await until(() => status() === "ready", "explicit original-owner check");
    check(
      f.calls.filter((call) => call.init.method === "PUT").length === 1 &&
        !f.calls.some((call) => call.url.includes("synchronizations")),
      "read replayed Open or prepared synchronization",
    );
    tests.push(
      "stale diff and native mismatch remain distinct refusals; explicit checks do not reload, reopen, refresh or fall back",
    );

    render(second.replace("-older", "-final"));
    await fileCalls(6);
    f.context.abort();
    await until(() => status() === "unavailable", "ended authority");
    files[5]!.reply.resolve(page(content.text));
    await new Promise<void>((resolve) => setTimeout(resolve, 20));
    check(
      count() === 8 && fileCount() === 6 && !latest?.diff.projection,
      "late file reply revived old authority",
    );
    tests.push(
      "ending actual injected core authority detaches a late file read, removes projection and forbids subsequent native I/O",
    );
  } finally {
    f.context.abort();
    flushSync(() => root.unmount());
    container.remove();
  }
  return tests;
}
