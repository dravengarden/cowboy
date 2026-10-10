import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import {
  CONVERSATION_SKELETON_TURNS,
  shouldPaintTranscriptLifecycle,
  transcriptLifecycleLabel,
  transcriptRestoreCaption,
} from "./transcriptLoadingPresentation.ts";

test("restore captions describe the load, never raw session status", () => {
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

test("clean exits stay out of the transcript", () => {
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

test("conversation skeletons mix assistant lines and user bubbles", () => {
  assert(CONVERSATION_SKELETON_TURNS.length >= 10);
  assert(CONVERSATION_SKELETON_TURNS.some((turn) => turn.mine));
  assert(CONVERSATION_SKELETON_TURNS.some((turn) => !turn.mine && turn.lines.length > 2));
});

test("transcript restore chrome uses the shared captions and wave skeletons", async () => {
  const transcript = await readFile(
    new URL("./Transcript.tsx", import.meta.url), "utf8",
  );
  const derive = await readFile(new URL("./derive.ts", import.meta.url), "utf8");
  assert(transcript.includes("transcriptRestoreCaption("));
  assert(transcript.includes("CONVERSATION_SKELETON_TURNS"));
  assert(transcript.includes('animation="wave"'));
  assertEquals(transcript.includes("Earlier conversation is available"), false);
  assertEquals(transcript.includes("Loading conversation data"), false);
  assert(derive.includes("shouldPaintTranscriptLifecycle("));
});
