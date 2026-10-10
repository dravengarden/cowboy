import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { Attachment } from "../attachments.ts";
import {
  DRAFT_ATTACHMENT_BYTES,
  DRAFT_ATTACHMENT_LIMIT,
  draftAttachmentsFit,
  type DraftMetadata,
  draftMissingDirectory,
} from "./model.ts";

function file(id: string, bytes: number): Attachment {
  return {
    id,
    name: `${id}.bin`,
    mimeType: "application/octet-stream",
    isImage: false,
    block: {
      type: "resource",
      resource: { uri: `file:///${id}`, blob: "a".repeat(bytes) },
    },
  } as Attachment;
}

function entry(
  id: string,
  parent_id: string | null,
  patch: Partial<DraftMetadata> = {},
): DraftMetadata {
  return {
    id,
    kind: "document",
    title: id,
    parent_id,
    revision: 1,
    body_revision: 1,
    metadata_revision: 1,
    updated_at_ms: 0,
    deleted: false,
    ...patch,
  };
}

test("attachments fit up to the server's count and size", () => {
  assertEquals(draftAttachmentsFit([]), true);
  assertEquals(draftAttachmentsFit([file("a", 1024)]), true);
  assertEquals(draftAttachmentsFit([file("a", DRAFT_ATTACHMENT_BYTES)]), false);
  assertEquals(
    draftAttachmentsFit([
      file("a", DRAFT_ATTACHMENT_BYTES / 2),
      file("b", DRAFT_ATTACHMENT_BYTES / 2),
    ]),
    false,
  );
  const many = Array.from(
    { length: DRAFT_ATTACHMENT_LIMIT + 1 },
    (_, index) => file(`f${index}`, 1),
  );
  assertEquals(draftAttachmentsFit(many.slice(1)), true);
  assertEquals(draftAttachmentsFit(many), false);
});

test("only a live document in a vanished directory is missing one", () => {
  const directories = new Set(["shared"]);
  assertEquals(draftMissingDirectory([entry("top", null)], directories), false);
  assertEquals(
    draftMissingDirectory([entry("placed", "shared")], directories),
    false,
  );
  assertEquals(
    draftMissingDirectory([entry("orphan", "gone")], directories),
    true,
  );
  assertEquals(
    draftMissingDirectory(
      [entry("trashed", "gone", { deleted: true })],
      directories,
    ),
    false,
  );
  // A legacy Draft folder is a directory of its own.
  assertEquals(
    draftMissingDirectory(
      [entry("legacy", null, { kind: "folder" }), entry("inside", "legacy")],
      directories,
    ),
    false,
  );
});
