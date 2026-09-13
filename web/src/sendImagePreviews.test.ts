import { assertEquals } from "jsr:@std/assert";
import type { Envelope } from "./protocol.ts";
import {
  confirmedImageSrc,
  promptEchoReadyToReplaceOptimistic,
  rememberSendImagePreviews,
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
