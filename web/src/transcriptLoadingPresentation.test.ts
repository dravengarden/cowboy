import { assert, assertEquals } from "jsr:@std/assert";
import {
  CONVERSATION_SKELETON_TURNS,
  shouldPaintTranscriptLifecycle,
  transcriptLifecycleLabel,
  transcriptRestoreCaption,
} from "./transcriptLoadingPresentation.ts";

Deno.test("restore captions describe the load, never raw session status", () => {
  assertEquals(transcriptRestoreCaption("hydrate"), "Restoring conversation");
  assertEquals(
    transcriptRestoreCaption("hydrate", "DeepSeek"),
    "Restoring DeepSeek conversation",
  );
  assertEquals(
    transcriptRestoreCaption("backfill"),
    "Loading earlier messages",
  );
  assertEquals(transcriptRestoreCaption("paused"), "Earlier messages");
});

Deno.test("clean exits stay out of the transcript", () => {
  assertEquals(shouldPaintTranscriptLifecycle("exited"), false);
  assertEquals(shouldPaintTranscriptLifecycle("running"), false);
  assertEquals(shouldPaintTranscriptLifecycle("crashed"), true);
  assertEquals(shouldPaintTranscriptLifecycle("interrupted"), true);
  assertEquals(
    transcriptLifecycleLabel("exited", null, (detail) => detail),
    null,
  );
  assertEquals(
    transcriptLifecycleLabel("crashed", "boom", (detail) => detail),
    "boom",
  );
});

Deno.test("conversation skeletons mix assistant lines and user bubbles", () => {
  assert(CONVERSATION_SKELETON_TURNS.length >= 10);
  assert(CONVERSATION_SKELETON_TURNS.some((turn) => turn.mine));
  assert(CONVERSATION_SKELETON_TURNS.some((turn) => !turn.mine && turn.lines.length > 2));
});

Deno.test("transcript restore chrome uses the shared captions and wave skeletons", async () => {
  const transcript = await Deno.readTextFile(
    new URL("./Transcript.tsx", import.meta.url),
  );
  const derive = await Deno.readTextFile(new URL("./derive.ts", import.meta.url));
  assert(transcript.includes("transcriptRestoreCaption("));
  assert(transcript.includes("CONVERSATION_SKELETON_TURNS"));
  assert(transcript.includes('animation="wave"'));
  assertEquals(transcript.includes("Earlier conversation is available"), false);
  assertEquals(transcript.includes("Loading conversation data"), false);
  assert(derive.includes("shouldPaintTranscriptLifecycle("));
});
