import { assertEquals } from "jsr:@std/assert";
import type { Attachment } from "./attachments.ts";
import { mergeRestoredDraft, storableAttachments } from "./draftRestore.ts";

function image(id: string, pending = false): Attachment {
  return {
    id,
    name: `${id}.png`,
    mimeType: "image/png",
    isImage: true,
    previewUrl: `data:image/png;base64,${id}`,
    block: { type: "image", data: id, mimeType: "image/png" },
    ...(pending ? { pending: true } : {}),
  };
}

Deno.test("a text mirror adopts the bytes the database kept for it", () => {
  const restored = mergeRestoredDraft(
    { text: "see ![a](cowboy-att:a) now", attachments: [], attachmentsInDatabase: true },
    { text: "see ![a](cowboy-att:a) old", attachments: [image("a")], savedAt: 1 },
  );
  assertEquals(restored?.text, "see ![a](cowboy-att:a) now");
  assertEquals(restored?.attachments.map((attachment) => attachment.id), ["a"]);
});

Deno.test("a mirror whose bytes are gone drops the orphaned tokens", () => {
  const restored = mergeRestoredDraft(
    { text: "see ![a](cowboy-att:a) now", attachments: [], attachmentsInDatabase: true },
    null,
  );
  assertEquals(restored?.text, "see now");
  assertEquals(restored?.attachments, []);
});

Deno.test("a complete mirror or a plain text draft needs no restore", () => {
  assertEquals(
    mergeRestoredDraft({ text: "plain", attachments: [] }, null),
    null,
  );
  assertEquals(
    mergeRestoredDraft(
      { text: "![a](cowboy-att:a)", attachments: [image("a")] },
      { text: "stale", attachments: [image("a")], savedAt: 1 },
    ),
    null,
  );
});

Deno.test("without a mirror the database record is the whole draft", () => {
  assertEquals(mergeRestoredDraft(undefined, null), null);
  const restored = mergeRestoredDraft(undefined, {
    text: "from db ![b](cowboy-att:b)",
    attachments: [image("b")],
    savedAt: 2,
  });
  assertEquals(restored?.text, "from db ![b](cowboy-att:b)");
  assertEquals(restored?.attachments.length, 1);
});

Deno.test("only completed attachments are stored", () => {
  assertEquals(
    storableAttachments([image("a"), image("b", true)]).map((attachment) => attachment.id),
    ["a"],
  );
});
