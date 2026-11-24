import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { assignMnemonics, isClippedOutsideScrollContainer } from "./hintTargets.ts";

test("mnemonics prefer explicit keys, then word initials, then letters", () => {
  assertEquals(
    assignMnemonics([
      { name: "Cancel" },
      { name: "Create session" },
      { name: "Session", preferred: "s" },
      { name: "Copy" },
    ]),
    ["c", "r", "s", "o"],
  );
});

test("mnemonics stay stable and unique across many controls", () => {
  const names = Array.from({ length: 40 }, (_, index) => ({ name: `Item ${index}` }));
  const first = assignMnemonics(names);
  assertEquals(first, assignMnemonics(names));
  const assigned = first.filter((key) => key !== null);
  assertEquals(new Set(assigned).size, assigned.length);
  assertEquals(assigned.length, 36);
});

test("CJK names fall back to free letters", () => {
  assertEquals(assignMnemonics([{ name: "创建" }, { name: "取消" }]), ["a", "s"]);
});

test("isClippedOutsideScrollContainer detects controls scrolled outside a clipping ancestor", () => {
  const origGetComputedStyle = globalThis.getComputedStyle;
  const origDocument = globalThis.document;
  const origInnerWidth = globalThis.innerWidth;
  const origInnerHeight = globalThis.innerHeight;
  const scrollBox = {
    parentElement: null,
    getBoundingClientRect: () => ({ top: 100, bottom: 300, left: 0, right: 400, width: 400, height: 200 }),
  };
  const visibleChild = {
    parentElement: scrollBox,
    getBoundingClientRect: () => ({ top: 140, bottom: 180, left: 20, right: 120, width: 100, height: 40 }),
  };
  const scrolledBelowChild = {
    parentElement: scrollBox,
    getBoundingClientRect: () => ({ top: 320, bottom: 360, left: 20, right: 120, width: 100, height: 40 }),
  };
  try {
    (globalThis as unknown as Record<string, unknown>).innerWidth = 1280;
    (globalThis as unknown as Record<string, unknown>).innerHeight = 800;
    (globalThis as unknown as Record<string, unknown>).document = { body: null };
    (globalThis as unknown as Record<string, unknown>).getComputedStyle = () => ({
      overflowY: "auto",
      overflowX: "visible",
    });
    assertEquals(
      isClippedOutsideScrollContainer(visibleChild as unknown as HTMLElement),
      false,
    );
    assertEquals(
      isClippedOutsideScrollContainer(scrolledBelowChild as unknown as HTMLElement),
      true,
    );
  } finally {
    globalThis.getComputedStyle = origGetComputedStyle;
    (globalThis as unknown as Record<string, unknown>).document = origDocument;
    (globalThis as unknown as Record<string, unknown>).innerWidth = origInnerWidth;
    (globalThis as unknown as Record<string, unknown>).innerHeight = origInnerHeight;
  }
});
