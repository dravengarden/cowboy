import { assertEquals } from "jsr:@std/assert";
import type { Attachment } from "../attachments.ts";
import { attachmentTrayForSurface } from "./attachmentPresentation.ts";

function attachment(id: string, isImage: boolean): Attachment {
  return {
    id,
    name: `${id}.bin`,
    mimeType: isImage ? "image/png" : "application/octet-stream",
    isImage,
    previewUrl: "",
    block: isImage
      ? { type: "image", data: "", mimeType: "image/png" }
      : { type: "resource", resource: { uri: `file:///${id}` } },
  };
}

Deno.test("compact composer keeps token-backed images inline", () => {
  const placed = attachment("placed", true);
  const staged = attachment("staged", true);
  const file = attachment("file", false);
  assertEquals(
    attachmentTrayForSurface(
      [placed, staged, file],
      "before ![shot](cowboy-att:placed) after",
    ).map((a) => a.id),
    ["staged", "file"],
  );
});

Deno.test("fullscreen keeps only images without CM inline tokens in its tray", () => {
  const placed = attachment("placed", true);
  const staged = attachment("staged", true);
  const file = attachment("file", false);
  assertEquals(
    attachmentTrayForSurface(
      [placed, staged, file],
      "before ![shot](cowboy-att:placed) after",
    ).map((a) => a.id),
    ["staged", "file"],
  );
});
