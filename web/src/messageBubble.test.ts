import { assert, assertEquals } from "jsr:@std/assert";
import {
  messageBubbleBorderRadius,
  messageBubbleLayoutSx,
} from "./messageBubble.ts";

Deno.test("only the speaker-side bottom corner is the small radius", () => {
  assertEquals(
    messageBubbleBorderRadius("user"),
    "18px 18px 6px 18px",
  );
  assertEquals(
    messageBubbleBorderRadius("assistant"),
    "18px 18px 18px 6px",
  );
});

Deno.test("long replies use the column; short sends stay compact", () => {
  const user = messageBubbleLayoutSx("user");
  const assistant = messageBubbleLayoutSx("assistant");
  assertEquals(user.width, "fit-content");
  assertEquals(user.maxWidth, "100%");
  assertEquals(user.alignSelf, "flex-end");
  assertEquals(assistant.width, "100%");
  assertEquals(assistant.alignSelf, "stretch");
});

Deno.test("confirmed and optimistic bubbles share the chat radius", async () => {
  const transcript = await Deno.readTextFile(
    new URL("./Transcript.tsx", import.meta.url),
  );
  const bubbleStart = transcript.indexOf("function OptimisticUserBubble(");
  const messageStart = transcript.indexOf("function MessageBubble(");
  const messageEnd = transcript.indexOf("function ToolCard(", messageStart);
  assert(bubbleStart >= 0 && messageStart > bubbleStart && messageEnd > messageStart);
  const optimistic = transcript.slice(bubbleStart, messageStart);
  const confirmed = transcript.slice(messageStart, messageEnd);
  assert(optimistic.includes('messageBubbleSurfaceSx("user"'));
  assert(optimistic.includes('messageBubbleLayoutSx("user"'));
  assert(confirmed.includes('messageBubbleSurfaceSx("user"'));
  assert(confirmed.includes('messageBubbleLayoutSx("user"'));
  assert(confirmed.includes('messageBubbleSurfaceSx("assistant"'));
  assert(confirmed.includes('messageBubbleLayoutSx("assistant"'));
  assertEquals(confirmed.includes('maxWidth: { xs: "88%"'), false);
});

Deno.test("a sent screenshot or file does not stretch the user bubble", async () => {
  const transcript = await Deno.readTextFile(
    new URL("./Transcript.tsx", import.meta.url),
  );
  const chip = await Deno.readTextFile(
    new URL("./TranscriptFileChip.tsx", import.meta.url),
  );
  // A percentage max-width is ignored while a fit-content bubble sizes itself;
  // the cap must be the pixel constant, on the element the bubble measures.
  assert(
    transcript.includes(
      'sx={{ maxWidth: MESSAGE_PREVIEW_MAX_WIDTH_PX, my: 0.5, mx: "auto" }}',
    ),
  );
  assert(chip.includes("maxWidth: MESSAGE_PREVIEW_MAX_WIDTH_PX,"));
  assertEquals(transcript.includes('maxWidth: "min(360px, 100%)"'), false);
  assertEquals(chip.includes('maxWidth: "min(360px, 100%)"'), false);
});
