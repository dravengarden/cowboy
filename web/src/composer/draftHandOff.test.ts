import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { Attachment } from "../attachments.ts";
import { getDraft, setDraft, subscribeDraftRestore } from "../draftStore.ts";
import { handOffSettledAttachments } from "./draftHandOff.ts";

function image(id: string, pending = false): Attachment {
  return {
    id,
    name: `${id}.png`,
    mimeType: "image/png",
    isImage: true,
    block: { type: "image", data: pending ? "" : "cGl4ZWw=", mimeType: "image/png" },
    ...(pending ? { pending: true } : {}),
  };
}

test("an image that finishes encoding after its composer was replaced reaches the draft", () => {
  const session = "hand-off-settled";
  // The replaced composer stored the text with the token; the bytes were not
  // ready, so the draft holds no attachment for it.
  setDraft(session, { text: "look ![a](cowboy-att:a) here", attachments: [] });
  const announced: string[][] = [];
  const stop = subscribeDraftRestore(session, (draft) => {
    announced.push(draft.attachments.map((attachment) => attachment.id));
  });
  handOffSettledAttachments(session, [image("a", true)], [image("a")]);
  stop();
  assertEquals(getDraft(session).text, "look ![a](cowboy-att:a) here");
  assertEquals(getDraft(session).attachments.map((attachment) => attachment.id), ["a"]);
  assertEquals(announced, [["a"]]);
});

test("a replaced composer never restores what its successor deleted", () => {
  const session = "hand-off-stale";
  // The successor removed image `old` and typed on. The replaced composer's
  // snapshot still lists it when its own paste settles.
  setDraft(session, { text: "typed since ![b](cowboy-att:b)", attachments: [] });
  handOffSettledAttachments(
    session,
    [image("old"), image("b", true)],
    [image("old"), image("b")],
  );
  assertEquals(getDraft(session).text, "typed since ![b](cowboy-att:b)");
  assertEquals(getDraft(session).attachments.map((attachment) => attachment.id), ["b"]);
});

test("an image that failed to encode takes only its own token with it", () => {
  const session = "hand-off-failed";
  setDraft(session, {
    text: "a  b ![x](cowboy-att:x) ![k](cowboy-att:k)",
    attachments: [image("k")],
  });
  handOffSettledAttachments(session, [image("k"), image("x", true)], [image("k")]);
  assertEquals(getDraft(session).text, "a  b ![k](cowboy-att:k)");
  assertEquals(getDraft(session).attachments.map((attachment) => attachment.id), ["k"]);
});

test("a hand-off with nothing newly settled leaves the draft alone", () => {
  const session = "hand-off-noop";
  const draft = { text: "untouched", attachments: [image("k")] };
  setDraft(session, draft);
  handOffSettledAttachments(session, [image("k")], [image("k")]);
  assertEquals(getDraft(session), draft);
});
