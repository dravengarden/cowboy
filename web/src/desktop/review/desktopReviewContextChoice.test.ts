import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { REVIEW_CONTEXT_CHOICE_LIMIT, withReviewContextChoice } from "./desktopReviewContextChoice.ts";

test("a Session's context choice is set, replaced and cleared within a bound", () => {
  let choices = withReviewContextChoice({}, "a", "project");
  choices = withReviewContextChoice(choices, "b", "session::x");
  choices = withReviewContextChoice(choices, "a", "session::y");
  assertEquals(Object.keys(choices), ["b", "a"]);
  assertEquals(withReviewContextChoice(choices, "a", null), { b: "session::x" });
  let many = {};
  for (let i = 0; i <= REVIEW_CONTEXT_CHOICE_LIMIT; i++) many = withReviewContextChoice(many, `s${String(i)}`, "project");
  assertEquals(Object.keys(many).length, REVIEW_CONTEXT_CHOICE_LIMIT);
  assertEquals("s0" in many, false);
});
