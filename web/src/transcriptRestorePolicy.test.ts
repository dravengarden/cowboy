import { assertEquals } from "jsr:@std/assert";
import {
  shouldInterruptTranscriptViewportRestore,
  shouldShowBlockingTranscriptRestore,
} from "./transcriptRestorePolicy.ts";

Deno.test("history restore blocks only when there is nothing useful to show", () => {
  assertEquals(shouldShowBlockingTranscriptRestore(true, 0, 0), true);
  assertEquals(shouldShowBlockingTranscriptRestore(true, 0, 1), false);
  assertEquals(shouldShowBlockingTranscriptRestore(true, 1, 0), false);
  assertEquals(shouldShowBlockingTranscriptRestore(false, 0, 0), false);
});

Deno.test("a newly submitted prompt interrupts a saved viewport restore", () => {
  assertEquals(shouldInterruptTranscriptViewportRestore(true, 1), true);
  assertEquals(shouldInterruptTranscriptViewportRestore(true, 0), false);
  assertEquals(shouldInterruptTranscriptViewportRestore(false, 1), false);
});
