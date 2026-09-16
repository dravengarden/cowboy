import { assert, assertEquals } from "jsr:@std/assert";
import { attachmentKindLabel } from "./TranscriptFileChip.tsx";

const transcript = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);

Deno.test("a sent file is labelled by its extension, then its MIME type", () => {
  assertEquals(attachmentKindLabel("2026 Product Release Slides.pdf"), "PDF");
  assertEquals(attachmentKindLabel("notes.md", "text/markdown"), "MD");
  assertEquals(attachmentKindLabel("report", "application/pdf"), "PDF");
  assertEquals(attachmentKindLabel("report", "text/plain; charset=utf-8"), "Text");
  assertEquals(attachmentKindLabel("blob"), "File");
});

Deno.test("confirmed and optimistic bubbles both show a file card", () => {
  assert(transcript.includes('if (chunk.type === "file") {'));
  assertEquals(transcript.includes("📎 {part.attachment.name}"), false);
  assert(transcript.includes("mimeType={part.attachment.mimeType}"));
});
