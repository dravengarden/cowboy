import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import {
  transcriptRowContainment,
  transcriptScrollerChildSx,
} from "./transcriptMotion.ts";
import { readTranscriptSource } from "./appSource.test.ts";

const transcriptSource = await readTranscriptSource();
const providerTranscriptSource = await readFile(
  new URL("./ProviderTranscript.tsx", import.meta.url), "utf8",
);
const markdownSource = await readFile(
  new URL("./MarkdownImpl.tsx", import.meta.url), "utf8",
);

test("thinking activity renders the exact Provider loading surface", () => {
  const component = transcriptSource.match(
    /function ThinkingIndicator[\s\S]*?(?=\n\/\/ Blinking text caret)/,
  )?.[0];
  assert(component);
  assert(component.includes("<ProviderRuntimeSurface"));
  assert(component.includes("providerVersion={providerVersion}"));
  assert(component.includes("providerDigest={providerDigest}"));
  assert(component.includes('slot="loading"'));
  assert(component.includes("activityKey"));
  assert(component.includes("waitingActivityLabel"));
  assert(component.includes("data-agent-waiting-label"));
  assert(transcriptSource.includes('data-transcript-tail-row="activity"'));
  assert(
    /data-transcript-tail-row="activity"\s+sx=\{\{\s*py: 0\.25/.test(
      transcriptSource,
    ),
  );
});

test("thinking activity has no Provider identity branches", () => {
  assertEquals(transcriptSource.includes("function ClaudeThinking"), false);
  assertEquals(transcriptSource.includes("function CodexThinking"), false);
  assertEquals(transcriptSource.includes("function GrokThinking"), false);
  assertEquals(transcriptSource.includes("providerActivityKind"), false);
  for (const provider of ["claude-code", "codex", "grok"]) {
    assertEquals(providerTranscriptSource.includes(`"${provider}"`), false);
  }
});

test("followed live tails recycle older rows into a height spacer", () => {
  assert(transcriptSource.includes("data-transcript-recycled-spacer"));
  assert(transcriptSource.includes("shouldWindowLiveTranscript"));
  assert(transcriptSource.includes("recycledTranscriptHeight"));
});

test("live-tail bookkeeping never synchronously measures a long Markdown row", () => {
  assert(transcriptSource.includes("new ResizeObserver((entries) =>"));
  assert(
    transcriptSource.includes(
      "if (!needsLiveTranscriptRowMeasurements(items.length)) return undefined",
    ),
  );
  assertEquals(
    transcriptSource.includes(
      "rowHeightsRef.current.set(key, node.offsetHeight)",
    ),
    false,
  );
});

test("repeated Markdown copy controls do not create backdrop blur layers", () => {
  const codeBlock = markdownSource.match(
    /function CodeBlock[\s\S]*?(?=\nclass MarkdownCodeBoundary)/,
  )?.[0];
  assert(codeBlock);
  assert(codeBlock.includes('className="cowboy-copy-btn"'));
  assertEquals(codeBlock.includes("backdropFilter"), false);
  assertEquals(codeBlock.includes("WebkitBackdropFilter"), false);
});

test("a streamed code block keeps one copy control until its fence closes", () => {
  // Keying the block by its content remounted the copy button on every chunk:
  // it flickered and a click (down and up on one element) never landed.
  assertEquals(markdownSource.includes("key={`${lang}:${text}`}"), false);
  assert(markdownSource.includes("<MarkdownCodeBoundary code={text} dark={dark}>"));
  assert(markdownSource.includes("fencedCodeIsOpen(renderedTextRef.current.slice(start, end))"));
  assert(markdownSource.includes("disabled={pending && !copied}"));
  assert(
    transcriptSource.includes(
      "streaming={!!streaming && i === lastChunkIdx}",
    ),
  );
});

test("drawer swipe does not React-render on transcript finger-down", () => {
  assert(transcriptSource.includes("renderPausedRef.current = true"));
  assert(
    transcriptSource.includes(
      "Do not unfollow on finger-down. A Sessions/Review swipe starts as a",
    ),
  );
  assert(
    transcriptSource.includes(
      "if (readerOwned && fromBottom > 1 && stick.current) detach()",
    ),
  );
  const touchStart = transcriptSource.indexOf("const onTouchStart = (): void => {");
  const detachInTouchStart = transcriptSource.indexOf(
    "detach();",
    touchStart,
  );
  const touchStartEnd = transcriptSource.indexOf(
    "const onTouchEnd = (): void => {",
    touchStart,
  );
  assert(touchStart >= 0 && touchStartEnd > touchStart);
  assert(detachInTouchStart === -1 || detachInTouchStart > touchStartEnd);
});

test("the growing row stays in the scroller paint flow", () => {
  assertEquals(transcriptRowContainment(true), "none");
  assertEquals(transcriptRowContainment(false), "layout paint");
  assertEquals(transcriptRowContainment(false, true), "layout");
  assertEquals(transcriptRowContainment(true, false, false), "none");
  assertEquals(transcriptRowContainment(false, false, false), "layout paint");
  assertEquals(transcriptRowContainment(false, true, false), "layout");
  // Desktop rows stay at constant layout containment so macOS WebKit never
  // toggles a row's stacking-context boundary on turn/tool settlement.
  assertEquals(transcriptRowContainment(true, false, true), "layout");
  assertEquals(transcriptRowContainment(false, false, true), "layout");
  assertEquals(transcriptRowContainment(false, true, true), "layout");
  assert(
    /contain:\s*transcriptRowContainment\(\s*item\.key === streamingRowKey \|\| isRunningTool\(item\),\s*item\.kind === "message" &&\s*item\.chunks\.some\(\(chunk\) => chunk\.type === "image"\),\s*desktopNavigation,\s*\)/
      .test(
        transcriptSource,
      ),
  );
});

test("restored visible scrollback resumes for each advanced cursor", () => {
  assert(
    transcriptSource.includes("shouldPrefetchVisibleScrollbackBoundary({"),
  );
  assert(
    transcriptSource.includes("scrollbackBoundaryRequestKey({"),
  );
  assert(
    /paging\?\.reachedStart,\s*scrollbackLoading,\s*sessionId,/.test(
      transcriptSource,
    ),
  );
  assert(
    transcriptSource.includes("VISIBLE_SCROLLBACK_SETTLE_FRAME_LIMIT = 120"),
  );
  assert(
    transcriptSource.includes(
      "visibleScrollbackBoundaryRafRef.current = requestAnimationFrame(measure)",
    ),
  );
  assertEquals(
    transcriptSource.match(
      /viewportRestoreActiveRef\.current = false;\s*(?:setMaskingViewportRestore\(false\);\s*)?requestVisibleScrollbackBoundaryRef\.current\(\);/g,
    )?.length,
    2,
  );
});

test("desktop transcript scroller contains every direct child at layout level without changing mobile", () => {
  assertEquals(transcriptScrollerChildSx(true), {
    flex: "0 0 auto",
    contain: "layout",
  });
  assertEquals(transcriptScrollerChildSx(false), {
    flex: "0 0 auto",
  });
  assert(
    transcriptSource.includes(
      '"& > *": transcriptScrollerChildSx(desktopNavigation),',
    ),
  );
});

test("transcript items do not allocate backdrop blur layers inside the scroller", async () => {
  const transcriptItemsSource = await readFile(
    new URL("./TranscriptItems.tsx", import.meta.url),
    "utf8",
  );
  assertEquals(transcriptItemsSource.includes("backdropFilter"), false);
  assertEquals(transcriptItemsSource.includes("WebkitBackdropFilter"), false);
});

test("desktop sessions list contains direct children at layout level", async () => {
  const sessionListSource = await readFile(
    new URL("./SessionList.tsx", import.meta.url),
    "utf8",
  );
  assert(
    /\.\.\.\(desktop && \{\s*(?:\/\/[^\n]*\n\s*)*"& > \*": \{ contain: "layout" \},/.test(
      sessionListSource,
    ),
  );
});
