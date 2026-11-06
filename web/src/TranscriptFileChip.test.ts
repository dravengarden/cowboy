import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import { attachmentKindLabel } from "./TranscriptFileChip.tsx";
import { readTranscriptSource } from "./appSource.test.ts";

const transcript = await readTranscriptSource();

test("a sent file is labelled by its extension, then its MIME type", () => {
  assertEquals(attachmentKindLabel("2026 Product Release Slides.pdf"), "PDF");
  assertEquals(attachmentKindLabel("notes.md", "text/markdown"), "MD");
  assertEquals(attachmentKindLabel("report", "application/pdf"), "PDF");
  assertEquals(attachmentKindLabel("report", "text/plain; charset=utf-8"), "Text");
  assertEquals(attachmentKindLabel("blob"), "File");
});

test("confirmed and optimistic bubbles both show a file card", () => {
  assert(transcript.includes('if (chunk.type === "file") {'));
  assertEquals(transcript.includes("📎 {part.attachment.name}"), false);
  assert(transcript.includes("mimeType={part.attachment.mimeType}"));
});
