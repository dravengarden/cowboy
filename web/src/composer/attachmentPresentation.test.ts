import { assertEquals } from "jsr:@std/assert";
import type { Attachment } from "../attachments.ts";
import { attachmentTrayForSurface } from "./attachmentPresentation.ts";

const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);

function attachment(id: string, isImage: boolean, previewUrl = "data:image/png;base64,c2hvdA=="): Attachment {
  return {
    id,
    name: `${id}.bin`,
    mimeType: isImage ? "image/png" : "application/octet-stream",
    isImage,
    previewUrl: isImage ? previewUrl : "",
    block: isImage
      ? { type: "image", data: "c2hvdA==", mimeType: "image/png" }
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

Deno.test("token-backed images with no loadable preview stay in the tray", () => {
  const placed = attachment("placed", true, "");
  const staged = attachment("staged", true);
  assertEquals(
    attachmentTrayForSurface(
      [placed, staged],
      "before ![shot](cowboy-att:placed) after",
    ).map((a) => a.id),
    ["placed", "staged"],
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

Deno.test("inline-image Preview dismisses the software keyboard before the lightbox", () => {
  const previewStart = composerSource.indexOf("Preview\n              </Button>");
  const preview = composerSource.slice(
    composerSource.lastIndexOf("<Button", previewStart),
    previewStart,
  );
  assertEquals(previewStart >= 0, true);
  assertEquals(preview.includes("noteMobileKeyboardDismissed();"), true);
  assertEquals(preview.includes("dismissMobileSoftwareKeyboard();"), true);
  assertEquals(preview.includes("releaseMobileComposerFocus();"), true);
  assertEquals(preview.includes("openLightbox([att], 0)"), true);
});

Deno.test("pending cards preview token-backed images inline instead of a second chip", () => {
  assertEquals(composerSource.includes("attachmentTrayForSurface(seedAttachments, seedText)"), true);
  assertEquals(composerSource.includes("<MessagePreview text={seedText} attachments={seedAttachments} />"), true);
});

Deno.test("pending edit surfaces retain previews for attachments without inline tokens", () => {
  assertEquals(
    composerSource.includes(
      "promoteUnplacedImageTokens(message.text, message.attachments)",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "const editTrayAttachments = attachmentTrayForSurface(\n      editAttachments,\n      draft,\n    );",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "touchInput && editTrayAttachments.length > 0",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "attachmentsSlot={editTrayAttachments.length > 0",
    ),
    true,
  );
  assertEquals(
    composerSource.match(/attachments={editTrayAttachments}/g)?.length,
    2,
  );
});

Deno.test("attachment-only pending rows retain a full content edit target", () => {
  assertEquals(
    composerSource.includes('data-pending-content-action="attachment-preview"'),
    true,
  );
  assertEquals(
    composerSource.includes(
      'sx={{ display: "inline-flex", flexWrap: "wrap", gap: 0.5, mt: 0.5 }}',
    ),
    true,
  );
  assertEquals(
    composerSource.includes("const pendingEditTap = useReliableTouchTap"),
    true,
  );
  assertEquals(
    composerSource.includes("data-pending-edit-target\n        {...pendingEditTap}"),
    true,
  );
  assertEquals(composerSource.includes("suppressPendingEditTap"), true);
  assertEquals(composerSource.includes("pendingContentCleared"), true);
  assertEquals(
    composerSource.includes(
      'target?.closest("[data-pending-content-action]")',
    ),
    false,
  );
  assertEquals(
    composerSource.includes(
      '"button, a, input, select, textarea, [role=\'button\'], [data-pending-content-action]"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes('data-pending-content-action="row-actions"'),
    true,
  );
  assertEquals(
    composerSource.includes('cursor: touchInput ? "pointer" : "text"'),
    true,
  );
});
