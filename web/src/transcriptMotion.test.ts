import { assert, assertEquals } from "jsr:@std/assert";
import { transcriptRowContainment } from "./transcriptMotion.ts";

const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("Codex activity copy fades without moving the text baseline", () => {
  const animation = transcriptSource.match(
    /const codexPhraseFade = keyframes`([\s\S]*?)`;/,
  )?.[1];
  assert(animation);
  assertEquals(animation.includes("translateY"), false);
});

Deno.test("Grok activity sweeps its provider mark without React churn", () => {
  const component = transcriptSource.match(
    /function GrokThinking[\s\S]*?(?=\nfunction DefaultThinking)/,
  )?.[0];
  assert(component);
  assert(transcriptSource.includes("const grokSignalSweep = keyframes"));
  assert(transcriptSource.includes("const GROK_SIGNAL_MS = 1800"));
  assert(component.includes('data-provider-activity="grok"'));
  assert(component.includes('className="grok-mark-base"'));
  assert(component.includes('className="grok-mark-signal"'));
  assert(component.includes("WebkitMaskImage"));
  assert(component.includes("Working"));
  assert(component.includes("prefers-reduced-motion: reduce"));
  assert(component.includes("width: 18"));
  assertEquals(component.includes("setInterval"), false);
  assertEquals(transcriptSource.includes("Grokking…"), false);
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
