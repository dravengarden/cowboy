import { assertEquals } from "jsr:@std/assert";
import type { Attachment } from "../attachments.ts";
import {
  nativeClipboardPlaceholderCount,
  runNativeClipboardImagePaste,
} from "./nativeClipboardImagePaste.ts";

Deno.test("native clipboard paste stages the selected range before reading bytes", async () => {
  const sequence: string[] = [];
  let releaseRead: ((files: File[]) => void) | undefined;
  const read = new Promise<File[]>((resolve) => {
    releaseRead = resolve;
  });
  let staged: Attachment[] = [];
  let completed: Attachment[] = [];
  const run = runNativeClipboardImagePaste(
    {
      expectedCount: 1,
      selection: { anchor: 7, head: 3 },
      read: () => {
        sequence.push("read");
        return read;
      },
    },
    {
      stage: (pending, selection): void => {
        sequence.push("stage");
        staged = pending;
        assertEquals(selection, { anchor: 7, head: 3 });
      },
      settle: (pending, resolved): void => {
        sequence.push("settle");
        assertEquals(
          pending.map((attachment) => attachment.id),
          staged.map((attachment) => attachment.id),
        );
        completed = resolved;
      },
    },
  );

  assertEquals(sequence, ["stage", "read"]);
  releaseRead?.([new File(["png"], "shot.png", { type: "image/png" })]);
  await run;
  assertEquals(sequence, ["stage", "read", "settle"]);
  assertEquals(completed.length, 1);
  assertEquals(completed[0]?.id, staged[0]?.id);
  assertEquals(completed[0]?.pending, undefined);
});

Deno.test("native clipboard placeholder count is bounded and never zero", () => {
  assertEquals(nativeClipboardPlaceholderCount(0), 1);
  assertEquals(nativeClipboardPlaceholderCount(Number.NaN), 1);
  assertEquals(nativeClipboardPlaceholderCount(3), 3);
  assertEquals(nativeClipboardPlaceholderCount(1000), 100);
});
