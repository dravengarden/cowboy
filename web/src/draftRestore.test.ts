import { test } from "bun:test";
import { assertEquals } from "@std/assert";
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

test("a text mirror adopts the bytes the database kept for it", () => {
  const restored = mergeRestoredDraft(
    { text: "see ![a](cowboy-att:a) now", attachments: [], attachmentsInDatabase: true },
    { text: "see ![a](cowboy-att:a) old", attachments: [image("a")], savedAt: 1 },
  );
  assertEquals(restored?.text, "see ![a](cowboy-att:a) now");
  assertEquals(restored?.attachments.map((attachment) => attachment.id), ["a"]);
});

test("a mirror whose bytes are gone drops the orphaned tokens", () => {
  const restored = mergeRestoredDraft(
    { text: "see ![a](cowboy-att:a) now", attachments: [], attachmentsInDatabase: true },
    null,
  );
  assertEquals(restored?.text, "see now");
  assertEquals(restored?.attachments, []);
});

test("a complete mirror or a plain text draft needs no restore", () => {
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

test("a mirror that promised nothing ignores a stale record", () => {
  // The record outlived a draft whose image was removed. Its bytes are not
  // part of what the user has now.
  assertEquals(
    mergeRestoredDraft(
      { text: "plain  text", attachments: [] },
      { text: "old ![a](cowboy-att:a)", attachments: [image("a")], savedAt: 1 },
    ),
    null,
  );
});

test("bytes return only for images the edited text still places", () => {
  const record = {
    text: "![a](cowboy-att:a) ![b](cowboy-att:b)",
    attachments: [image("a"), image("b")],
    savedAt: 1,
  };
  const restored = mergeRestoredDraft(
    { text: "kept ![b](cowboy-att:b) and more", attachments: [], attachmentsInDatabase: true },
    record,
    "![a](cowboy-att:a) ![b](cowboy-att:b)",
  );
  assertEquals(restored?.text, "kept ![b](cowboy-att:b) and more");
  assertEquals(restored?.attachments.map((attachment) => attachment.id), ["b"]);
});

test("an image pasted while the bytes were promised is kept beside them", () => {
  const restored = mergeRestoredDraft(
    {
      text: "![a](cowboy-att:a) new ![c](cowboy-att:c)",
      attachments: [image("c")],
      attachmentsInDatabase: true,
    },
    { text: "![a](cowboy-att:a)", attachments: [image("a")], savedAt: 1 },
    "![a](cowboy-att:a)",
  );
  assertEquals(restored?.text, "![a](cowboy-att:a) new ![c](cowboy-att:c)");
  assertEquals(restored?.attachments.map((attachment) => attachment.id), ["c", "a"]);
});

test("restoring never rewrites the spacing the user typed", () => {
  const text = "    code  with   runs ![a](cowboy-att:a)\n\tand a tab";
  const restored = mergeRestoredDraft(
    { text, attachments: [], attachmentsInDatabase: true },
    { text: "", attachments: [image("a")], savedAt: 1 },
  );
  assertEquals(restored?.text, text);
});

test("without a mirror the database record is the whole draft", () => {
  assertEquals(mergeRestoredDraft(undefined, null), null);
  const restored = mergeRestoredDraft(undefined, {
    text: "from db ![b](cowboy-att:b)",
    attachments: [image("b")],
    savedAt: 2,
  });
  assertEquals(restored?.text, "from db ![b](cowboy-att:b)");
  assertEquals(restored?.attachments.length, 1);
});

test("only completed attachments are stored", () => {
  assertEquals(
    storableAttachments([image("a"), image("b", true)]).map((attachment) => attachment.id),
    ["a"],
  );
});
