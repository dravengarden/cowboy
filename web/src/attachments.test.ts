import { assertEquals } from "jsr:@std/assert";
import {
  type Attachment,
  reconcileDeletedInlineImages,
} from "./attachments.ts";

function attachment(id: string, isImage: boolean): Attachment {
  return {
    id,
    name: `${id}.png`,
    mimeType: isImage ? "image/png" : "text/plain",
    isImage,
    block: { type: isImage ? "image" : "resource" },
  };
}

Deno.test("deleted inline tokens remove only their image attachments", () => {
  const inline = attachment("inline", true);
  const gallery = attachment("gallery", true);
  const file = attachment("file", false);
  const previous = "before\n![shot](cowboy-att:inline)\nafter";

  assertEquals(
    reconcileDeletedInlineImages(previous, "before\nafter", [inline, gallery, file]),
    [gallery, file],
  );
});

Deno.test("an image attachment remains while any matching token remains", () => {
  const inline = attachment("inline", true);
  const previous = "![one](cowboy-att:inline)\n![two](cowboy-att:inline)";
  const next = "![two](cowboy-att:inline)";

  assertEquals(reconcileDeletedInlineImages(previous, next, [inline]), [inline]);
});
