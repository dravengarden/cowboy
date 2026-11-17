import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import {
  MAX_CLIPS,
  MAX_CLIP_CHARS,
  NO_CLIPS,
  addClip,
  clipLength,
  clipPreview,
  offeredClip,
  settleOffer,
} from "./remoteClips";

test("each new remote copy is held newest first and offered once", () => {
  const one = addClip(NO_CLIPS, "code-1", 10);
  assertEquals(offeredClip(one)?.text, "code-1");
  const two = addClip(one, "验证码 42 🎉", 20);
  assertEquals(
    two.items.map((clip) => clip.text),
    ["验证码 42 🎉", "code-1"],
  );
  assertEquals(offeredClip(two)?.text, "验证码 42 🎉");
  const settled = settleOffer(two, two.offer!);
  assertEquals(offeredClip(settled), undefined);
  // Settling an older offer leaves a newer one alone.
  assertEquals(settleOffer(two, one.offer!), two);
});

test("a re-announced clipboard changes nothing; an older duplicate moves up", () => {
  const held = addClip(addClip(NO_CLIPS, "a", 1), "b", 2);
  const settled = settleOffer(held, held.offer!);
  assertEquals(addClip(settled, "b", 3), settled);
  assertEquals(addClip(settled, "", 3), settled);
  const again = addClip(settled, "a", 4);
  assertEquals(
    again.items.map((clip) => clip.text),
    ["a", "b"],
  );
  assertEquals(offeredClip(again)?.at, 4);
  assertEquals(new Set(again.items.map((clip) => clip.id)).size, 2);
});

test("history is bounded by count and total size, keeping the newest whole", () => {
  let clips = NO_CLIPS;
  for (let index = 0; index < MAX_CLIPS + 5; index += 1) clips = addClip(clips, `t${index}`, index);
  assertEquals(clips.items.length, MAX_CLIPS);
  assertEquals(clips.items[0]?.text, `t${MAX_CLIPS + 4}`);
  const huge = "x".repeat(MAX_CLIP_CHARS + 1);
  const alone = addClip(clips, huge, 99);
  assertEquals(alone.items.length, 1);
  assertEquals(alone.items[0]?.text.length, MAX_CLIP_CHARS + 1);
});

test("previews collapse whitespace and never split a character", () => {
  assertEquals(clipPreview("  a\n\n b\tc "), "a b c");
  assertEquals(clipPreview("🎉".repeat(5), 3), "🎉🎉🎉…");
  assertEquals(clipLength("🎉a"), 2);
});
