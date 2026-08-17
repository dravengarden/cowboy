import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import {
  attachmentDisplayParts,
  type Attachment,
  clipboardFiles,
  pendingClipboardImageAttachment,
  IMG_TOKEN_RE,
  promoteUnplacedImageTokens,
  reconcileDeletedInlineImages,
  settlePendingAttachments,
  stripImageTokens,
} from "./attachments.ts";

Deno.test("native clipboard placeholders reserve image ids without fake bytes", () => {
  const pending = pendingClipboardImageAttachment(1, "clipboard-two");
  assertEquals(pending, {
    id: "clipboard-two",
    name: "pasted-image-2.png",
    mimeType: "image/png",
    isImage: true,
    block: { type: "image", data: "", mimeType: "image/png" },
    pending: true,
  });
});

Deno.test("clipboard files include iOS item-only images without duplicates", () => {
  const direct = new File(["direct"], "direct.png", { type: "image/png" });
  const itemOnly = new File(["item"], "item.png", { type: "image/png" });
  const clipboard = {
    files: [direct],
    items: [
      { kind: "file", getAsFile: () => direct },
      { kind: "file", getAsFile: () => itemOnly },
      { kind: "string", getAsFile: () => null },
    ],
  } as unknown as Pick<DataTransfer, "files" | "items">;
  assertEquals(clipboardFiles(clipboard), [direct, itemOnly]);
});

function attachment(id: string, isImage: boolean): Attachment {
  return {
    id,
    name: `${id}.png`,
    mimeType: isImage ? "image/png" : "text/plain",
    isImage,
    block: { type: isImage ? "image" : "resource" },
  };
}

Deno.test("settling one image paste batch preserves newer pending images", () => {
  const firstPending = { ...attachment("first", true), pending: true };
  const secondPending = { ...attachment("second", true), pending: true };
  const firstCompleted = attachment("first", true);
  const existing = attachment("existing", false);

  assertEquals(
    settlePendingAttachments(
      [existing, firstPending, secondPending],
      [firstPending],
      [firstCompleted],
    ),
    [existing, firstCompleted, secondPending],
  );
});

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

  const attachments = [inline];
  assertStrictEquals(
    reconcileDeletedInlineImages(previous, next, attachments),
    attachments,
  );
});

Deno.test("ordinary text input preserves attachment state identity", () => {
  const attachments = [attachment("file", false)];
  assertStrictEquals(
    reconcileDeletedInlineImages("hello", "hello!", attachments),
    attachments,
  );
});

Deno.test("legacy unplaced images regain deterministic inline positions", () => {
  const placed = attachment("placed", true);
  const legacy = attachment("legacy", true);
  const file = attachment("notes", false);
  assertEquals(
    promoteUnplacedImageTokens(
      "before\n![placed.png](cowboy-att:placed)\nafter",
      [placed, legacy, file],
    ),
    "before\n![placed.png](cowboy-att:placed)\nafter\n" +
      "![legacy.png](cowboy-att:legacy)\n",
  );
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

Deno.test("stripImageTokens hides cowboy-att source even after a prior global scan", () => {
  const token =
    "![pasted-image-1.png](cowboy-att:att-f6da137a-d97f-4f24-bcfd-36945ab21a3d)";
  IMG_TOKEN_RE.lastIndex = token.length;
  assertEquals(
    stripImageTokens(`${token}\n移动端这里没有触发 Obsidian 的渲染吧？`),
    "\n移动端这里没有触发 Obsidian 的渲染吧？",
  );
});

Deno.test("local attachment-only messages render the attachment, not fallback text", () => {
  const image = attachment("shot", true);
  image.previewUrl = "data:image/png;base64,c2hvdA==";

  assertEquals(attachmentDisplayParts("", [image]), [
    { type: "attachment", attachment: image },
  ]);
});
