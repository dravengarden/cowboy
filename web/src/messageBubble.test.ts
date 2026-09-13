import { assert, assertEquals } from "jsr:@std/assert";
import { messageBubbleBorderRadius } from "./messageBubble.ts";

Deno.test("user and assistant bubbles mirror left/right corner radii", () => {
  assertEquals(
    messageBubbleBorderRadius("user"),
    "18px 6px 6px 18px",
  );
  assertEquals(
    messageBubbleBorderRadius("assistant"),
    "6px 18px 18px 6px",
  );
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
  assert(confirmed.includes('messageBubbleSurfaceSx("user"'));
  assert(confirmed.includes('messageBubbleSurfaceSx("assistant"'));
});
