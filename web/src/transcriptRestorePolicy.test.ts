import { assertEquals } from "jsr:@std/assert";
import {
  hasNewOptimisticDelivery,
  shouldInterruptTranscriptViewportRestore,
  shouldShowBlockingTranscriptRestore,
} from "./transcriptRestorePolicy.ts";

Deno.test("history restore blocks only when there is nothing useful to show", () => {
  assertEquals(shouldShowBlockingTranscriptRestore(true, 0, 0), true);
  assertEquals(shouldShowBlockingTranscriptRestore(true, 0, 1), false);
  assertEquals(shouldShowBlockingTranscriptRestore(true, 1, 0), false);
  assertEquals(shouldShowBlockingTranscriptRestore(false, 0, 0), false);
});

Deno.test("a just-sent prompt keeps the restore skeleton from covering it", async () => {
  const transcript = await Deno.readTextFile(
    new URL("./Transcript.tsx", import.meta.url),
  );
  const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  assertEquals(
    transcript.includes("pendingMessages.length"),
    true,
  );
  assertEquals(
    transcript.includes("matched.length > 0 ? matched : pendingMessages"),
    true,
  );
  assertEquals(
    store.includes("cmid !== undefined && cached &&"),
    true,
  );
});

Deno.test("a newly submitted prompt interrupts a saved viewport restore", () => {
  assertEquals(shouldInterruptTranscriptViewportRestore(true, 1), true);
  assertEquals(shouldInterruptTranscriptViewportRestore(true, 0), false);
  assertEquals(shouldInterruptTranscriptViewportRestore(false, 1), false);
});

Deno.test("a fresh local delivery is detected even when an echo keeps the count stable", () => {
  assertEquals(hasNewOptimisticDelivery(["old"], ["new"]), true);
  assertEquals(hasNewOptimisticDelivery(["old"], ["old", "new"]), true);
  assertEquals(hasNewOptimisticDelivery(["old"], ["old"]), false);
  assertEquals(hasNewOptimisticDelivery(["old"], []), false);
});
