import { assertEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import {
  confirmedImageSrc,
  envelopeCompletesPromptEcho,
  promptEchoReadyToReplaceOptimistic,
  promptEchoText,
  rememberSendImagePreviews,
  retainUnpresentedOptimistic,
} from "./sendImagePreviews.ts";

function envelope(
  seq: number,
  type: "image" | "text",
  cmid?: string,
): Envelope {
  return {
    session_id: "s1",
    seq,
    kind: "update",
    ...(cmid !== undefined ? { cmid } : {}),
    update: {
      sessionUpdate: "user_message_chunk",
      content: type === "image"
        ? { type: "image", url: "/api/artifacts/shot.jpg" }
        : { type: "text", text: "caption" },
    },
  };
}

const imageMessage = {
  cmid: "c1",
  attachments: [{ isImage: true }],
};

Deno.test("text-first echoes keep the optimistic image until the image block lands", () => {
  assertEquals(
    promptEchoReadyToReplaceOptimistic(imageMessage, [
      envelope(1, "text", "c1"),
    ]),
    false,
  );
  assertEquals(
    promptEchoReadyToReplaceOptimistic(imageMessage, [
      envelope(1, "text", "c1"),
      envelope(2, "image"),
    ]),
    true,
  );
});

Deno.test("image-first echoes can replace once the image block is present", () => {
  assertEquals(
    promptEchoReadyToReplaceOptimistic(imageMessage, [
      envelope(1, "image", "c1"),
    ]),
    true,
  );
});

Deno.test("text-only sends replace on the first tagged echo", () => {
  assertEquals(
    promptEchoReadyToReplaceOptimistic({ cmid: "c1", attachments: [] }, [
      envelope(1, "text", "c1"),
    ]),
    true,
  );
});

Deno.test("a tagged but unrenderable image echo cannot replace the overlay", () => {
  const emptyImage: Envelope = {
    session_id: "s1",
    seq: 1,
    cmid: "c1",
    kind: "update",
    update: {
      sessionUpdate: "user_message_chunk",
      content: { type: "image" },
    },
  };
  assertEquals(
    promptEchoReadyToReplaceOptimistic({ cmid: "c1", attachments: [] }, [
      emptyImage,
    ]),
    false,
  );
  assertEquals(
    promptEchoReadyToReplaceOptimistic(imageMessage, [emptyImage]),
    false,
  );
});

Deno.test("unpresented overlays linger until the presented timeline can replace them", () => {
  const overlay = { cmid: "c1", attachments: [] as { isImage?: boolean }[] };
  assertEquals(
    retainUnpresentedOptimistic([overlay], [], [], true),
    [overlay],
  );
  assertEquals(
    retainUnpresentedOptimistic([overlay], [], [
      envelope(1, "text", "c1"),
    ], true),
    [],
  );
  assertEquals(
    retainUnpresentedOptimistic([overlay], [overlay], [
      envelope(1, "text", "c1"),
    ], true),
    [overlay],
  );
});

Deno.test("a live presentation follows the store even when the echo lost its cmid", () => {
  // Echoes persisted before the submission ledger carry no cmid. Once the renderer
  // shows the canonical timeline, a retired overlay must not linger and hide
  // the next human prompt behind a repainted copy of this one.
  const overlay = { cmid: "c1", attachments: [{ isImage: true }] };
  const untagged = [envelope(1, "image"), envelope(2, "text")];
  assertEquals(retainUnpresentedOptimistic([overlay], [], untagged, true), [
    overlay,
  ]);
  assertEquals(retainUnpresentedOptimistic([overlay], [], untagged, false), []);
});

Deno.test("only agent work after the echo proves the whole prompt was echoed", () => {
  const update = (sessionUpdate: string): Envelope => ({
    session_id: "s1",
    seq: 3,
    kind: "update",
    update: { sessionUpdate },
  });
  assertEquals(envelopeCompletesPromptEcho(envelope(2, "text")), false);
  assertEquals(
    envelopeCompletesPromptEcho({
      session_id: "s1",
      seq: 3,
      kind: "lifecycle",
      status: "busy",
      detail: null,
    }),
    false,
  );
  assertEquals(envelopeCompletesPromptEcho(update("usage_update")), false);
  assertEquals(envelopeCompletesPromptEcho(update("agent_message_chunk")), true);
  assertEquals(envelopeCompletesPromptEcho(update("tool_call")), true);
  assertEquals(
    envelopeCompletesPromptEcho({
      session_id: "s1",
      seq: 3,
      kind: "turn_end",
      stop_reason: "Cancelled",
    }),
    true,
  );
});

Deno.test("confirmed image rows keep the send preview instead of the artifact URL", () => {
  rememberSendImagePreviews("c1", [{
    id: "att-1",
    name: "shot.jpg",
    mimeType: "image/jpeg",
    isImage: true,
    previewUrl: "data:image/jpeg;base64,c2hvdA==",
    block: { type: "image", data: "c2hvdA==", mimeType: "image/jpeg" },
  }]);
  assertEquals(
    confirmedImageSrc("/api/artifacts/shot.jpg", "c1", 0),
    "data:image/jpeg;base64,c2hvdA==",
  );
  assertEquals(
    confirmedImageSrc("/api/artifacts/other.jpg", "missing", 0),
    "/api/artifacts/other.jpg",
  );
});

Deno.test("an adjacent send cannot supply the missing image of an earlier echo", () => {
  assertEquals(promptEchoReadyToReplaceOptimistic(imageMessage, [
    envelope(1, "text", "c1"),
    envelope(2, "image", "c2"),
  ]), false);
});

Deno.test("image-first captions and files remain on the local bubble until all content is present", () => {
  const message = { cmid: "c1", text: "caption", attachments: [{ isImage: true }, { isImage: false }] };
  const image = envelope(1, "image", "c1");
  const caption = envelope(3, "text");
  const file: Envelope = {
    session_id: "s1", seq: 4, kind: "update",
    update: { sessionUpdate: "user_message_chunk", content: { type: "resource_link", uri: "file:///notes.txt", name: "notes.txt" } },
  };
  const busy: Envelope = { session_id: "s1", seq: 2, kind: "lifecycle", status: "busy", detail: null };
  assertEquals(promptEchoReadyToReplaceOptimistic(message, [image]), false);
  assertEquals(promptEchoReadyToReplaceOptimistic(message, [image, busy, caption]), false);
  assertEquals(promptEchoReadyToReplaceOptimistic(message, [image, busy, caption, file]), true);
});

Deno.test("confirmed previews follow inline document order instead of attachment tray order", () => {
  const image = (id: string, data: string) => ({
    id, name: `${id}.png`, mimeType: "image/png", isImage: true,
    previewUrl: `data:image/png;base64,${data}`,
    block: { type: "image" as const, data, mimeType: "image/png" },
  });
  rememberSendImagePreviews("reordered", [image("first", "YQ=="), image("second", "Yg==")],
    "![second](cowboy-att:second) caption ![first](cowboy-att:first)");
  assertEquals(confirmedImageSrc("/artifact/1", "reordered", 0), "data:image/png;base64,Yg==");
  assertEquals(confirmedImageSrc("/artifact/2", "reordered", 1), "data:image/png;base64,YQ==");
});

Deno.test("the echoed text of a tagged prompt is recovered for draft retirement", () => {
  const events = [envelope(1, "text", "c1"), envelope(2, "image", "c1"), envelope(3, "text", "c2")];
  assertEquals(promptEchoText(events, "c1"), "caption");
  assertEquals(promptEchoText(events, "missing"), undefined);
});
