import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  DESKTOP_WORK_VIEW_LIMIT,
  desktopWorkViewFromEvent,
  loadDesktopWorkView,
  saveDesktopWorkView,
  storedReviewSessions,
} from "./desktopWorkView.ts";

test("only Review is stored, newest change last, within the limit", () => {
  assertEquals(storedReviewSessions(["a", "b"], "a", "review"), ["b", "a"]);
  assertEquals(storedReviewSessions(["a", "b"], "a", "conversation"), ["b"]);
  const many = Array.from({ length: DESKTOP_WORK_VIEW_LIMIT }, (_, i) => `s${String(i)}`);
  const next = storedReviewSessions(many, "new", "review");
  assertEquals(next.length, DESKTOP_WORK_VIEW_LIMIT);
  assertEquals(next[0], "s1");
  assertEquals(next.at(-1), "new");
});

test("a Session remembers its view on this device", () => {
  saveDesktopWorkView("one", "review");
  assertEquals(loadDesktopWorkView("one"), "review");
  assertEquals(loadDesktopWorkView("two"), "conversation");
  saveDesktopWorkView("one", "conversation");
  assertEquals(loadDesktopWorkView("one"), "conversation");
});

test("work view requests carry only a known view", () => {
  assertEquals(desktopWorkViewFromEvent(new CustomEvent("x", { detail: "review" })), "review");
  assertEquals(desktopWorkViewFromEvent(new CustomEvent("x", { detail: "files" })), null);
});
