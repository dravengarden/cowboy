import { assertEquals } from "jsr:@std/assert";
import {
  attachmentDisplayParts,
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

Deno.test("local message display keeps image bytes at their inline position", () => {
  const image = attachment("shot", true);
  image.previewUrl = "data:image/png;base64,c2hvdA==";
  const file = attachment("notes", false);

  assertEquals(
    attachmentDisplayParts(
      "before\n![image.png](cowboy-att:shot)\nafter",
      [image, file],
    ),
    [
      { type: "text", text: "before\n" },
      { type: "attachment", attachment: image },
      { type: "text", text: "\nafter" },
      { type: "attachment", attachment: file },
    ],
  );
});

Deno.test("local attachment-only messages render the attachment, not fallback text", () => {
  const image = attachment("shot", true);
  image.previewUrl = "data:image/png;base64,c2hvdA==";

  assertEquals(attachmentDisplayParts("", [image]), [
    { type: "attachment", attachment: image },
  ]);
});
