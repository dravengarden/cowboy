import { assert, assertEquals } from "jsr:@std/assert";
import { transcriptRowContainment } from "./transcriptMotion.ts";

const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
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
});

Deno.test("thinking activity has no Provider identity branches", () => {
  assertEquals(transcriptSource.includes("function ClaudeThinking"), false);
  assertEquals(transcriptSource.includes("function CodexThinking"), false);
  assertEquals(transcriptSource.includes("function GrokThinking"), false);
  assertEquals(transcriptSource.includes("providerActivityKind"), false);
});

Deno.test("the growing row stays in the scroller paint flow", () => {
  assertEquals(transcriptRowContainment(true), "none");
  assertEquals(transcriptRowContainment(false), "layout paint");
  assert(
    /contain:\s*transcriptRowContainment\(\s*item\.key === streamingRowKey/
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
      /viewportRestoreActiveRef\.current = false;\s*requestVisibleScrollbackBoundaryRef\.current\(\);/g,
    )?.length,
    2,
  );
});
