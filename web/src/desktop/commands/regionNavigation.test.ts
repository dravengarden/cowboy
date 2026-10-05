import { assertEquals } from "jsr:@std/assert";
import { regionInDirection, regionMotionKey } from "./regionNavigation.ts";

const box = (left: number, top: number, right: number, bottom: number) => ({
  left,
  top,
  right,
  bottom,
});

// Sessions | Prompt (composer over queue) | Conversation, a top bar above
// Prompt and Conversation.
const sessions = box(0, 0, 260, 1000);
const topbar = box(260, 0, 1560, 40);
const composer = box(260, 40, 820, 800);
const queue = box(260, 800, 820, 1000);
const conversation = box(820, 40, 1560, 1000);
const all = [sessions, topbar, composer, queue, conversation];

Deno.test("Ctrl+H/L cross panes by geometry", () => {
  assertEquals(regionInDirection(composer, all, "h"), 0);
  assertEquals(regionInDirection(composer, all, "l"), 4);
  assertEquals(regionInDirection(queue, all, "l"), 4);
  assertEquals(regionInDirection(conversation, all, "h"), 2);
  assertEquals(regionInDirection(sessions, all, "h"), null);
  assertEquals(regionInDirection(conversation, all, "l"), null);
});

Deno.test("Ctrl+J/K move within a column and up to the top bar", () => {
  assertEquals(regionInDirection(composer, all, "j"), 3);
  assertEquals(regionInDirection(queue, all, "k"), 2);
  assertEquals(regionInDirection(composer, all, "k"), 1);
  assertEquals(regionInDirection(conversation, all, "k"), 1);
  // From the full-width top bar J prefers Prompt, the first column under it;
  // with Prompt folded it reaches Conversation.
  assertEquals(regionInDirection(topbar, all, "j"), 2);
  assertEquals(
    regionInDirection(topbar, [sessions, topbar, conversation], "j"),
    2,
  );
  assertEquals(regionInDirection(queue, all, "j"), null);
});

Deno.test("only plain Ctrl with h/j/k/l by physical key", () => {
  const event = (code: string, extra = {}) => ({
    code,
    ctrlKey: true,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...extra,
  });
  assertEquals(regionMotionKey(event("KeyL")), "l");
  assertEquals(regionMotionKey(event("KeyH")), "h");
  assertEquals(regionMotionKey(event("KeyL", { shiftKey: true })), null);
  assertEquals(regionMotionKey(event("KeyL", { metaKey: true })), null);
  assertEquals(regionMotionKey(event("KeyA")), null);
});
