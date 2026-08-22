import { assert, assertEquals } from "jsr:@std/assert";
import { transcriptRowContainment } from "./transcriptMotion.ts";

const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);
const providerTranscriptSource = await Deno.readTextFile(
  new URL("./ProviderTranscript.tsx", import.meta.url),
);

Deno.test("thinking activity renders the exact Provider loading surface", () => {
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

Deno.test("thinking activity has no Provider identity branches", () => {
  assertEquals(transcriptSource.includes("function ClaudeThinking"), false);
  assertEquals(transcriptSource.includes("function CodexThinking"), false);
  assertEquals(transcriptSource.includes("function GrokThinking"), false);
  assertEquals(transcriptSource.includes("providerActivityKind"), false);
  for (const provider of ["claude-code", "codex", "gemini", "grok"]) {
    assertEquals(providerTranscriptSource.includes(`"${provider}"`), false);
  }
});

Deno.test("followed live tails recycle older rows into a height spacer", () => {
  assert(transcriptSource.includes("data-transcript-recycled-spacer"));
  assert(transcriptSource.includes("shouldWindowLiveTranscript"));
  assert(transcriptSource.includes("recycledTranscriptHeight"));
});

Deno.test("drawer swipe does not React-render on transcript finger-down", () => {
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

Deno.test("the growing row stays in the scroller paint flow", () => {
  assertEquals(transcriptRowContainment(true), "none");
  assertEquals(transcriptRowContainment(false), "layout paint");
  assertEquals(transcriptRowContainment(false, true), "layout");
  assert(
    /contain:\s*transcriptRowContainment\(\s*item\.key === streamingRowKey,\s*item\.kind === "message" &&\s*item\.chunks\.some\(\(chunk\) => chunk\.type === "image"\)/
      .test(
        transcriptSource,
      ),
  );
});

Deno.test("restored visible scrollback resumes for each advanced cursor", () => {
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
