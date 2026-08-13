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

Deno.test("the growing row stays in the scroller paint flow", () => {
  assertEquals(transcriptRowContainment(true), "none");
  assertEquals(transcriptRowContainment(false), "layout paint");
  assert(
    /contain:\s*transcriptRowContainment\(\s*item\.key === streamingRowKey/.test(
      transcriptSource,
    ),
  );
});

Deno.test("restored visible scrollback resumes without a touch gesture", () => {
  assert(
    transcriptSource.includes("shouldPrefetchVisibleScrollbackBoundary({"),
  );
  assertEquals(
    transcriptSource.match(
      /viewportRestoreActiveRef\.current = false;\s*requestVisibleScrollbackBoundaryRef\.current\(\);/g,
    )?.length,
    2,
  );
});
