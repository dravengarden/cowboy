import { assertEquals } from "jsr:@std/assert";
import type { Attachment } from "../attachments.ts";
import { attachmentTrayForSurface } from "./attachmentPresentation.ts";

const composerSource = await Deno.readTextFile(
  new URL("../Composer.tsx", import.meta.url),
);

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
    /<Paper[\s\S]*?data-pending-edit-target[\s\S]*?\{\.\.\.pendingEditTap\}/
      .test(composerSource),
    true,
  );
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
