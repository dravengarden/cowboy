import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { readFile } from "node:fs/promises";
import {
  COMPOSER_STACK_DISCLOSURE_TIMEOUT,
  swallowStrayDisclosureClick,
} from "./composerStackDisclosure.ts";

const source = (name: string): Promise<string> =>
  readFile(new URL(`./${name}`, import.meta.url), "utf8");

test("composer-stack disclosures change height in one step", async () => {
  // A height transition relaid the Transcript on every frame of the 300ms
  // disclosure; the one-step height is the fix for the expand/collapse jank.
  assertEquals(COMPOSER_STACK_DISCLOSURE_TIMEOUT, 0);
  const expected: Record<string, number> = {
    "Composer.tsx": 1, // Queue / Drafts
    "PlanDock.tsx": 2, // Mobile + Desktop Plan
    "ManagedCallsDock.tsx": 1, // Calls
  };
  for (const [file, count] of Object.entries(expected)) {
    const text = await source(file);
    assertEquals(
      text.split("timeout={COMPOSER_STACK_DISCLOSURE_TIMEOUT}").length - 1,
      count,
      file,
    );
    // Every touch toggle arms the stray-click guard for the moved layout.
    assertEquals(text.includes("swallowStrayDisclosureClick,\n  );"), true, file);
  }
});

function click(detail: number, pointerType: string): Event {
  return Object.assign(new Event("click", { cancelable: true }), {
    detail,
    pointerType,
  });
}

test("the stray-click guard swallows only the paired touch click", () => {
  const original = globalThis.document;
  const doc = new EventTarget();
  Object.assign(globalThis, { document: doc });
  try {
    // A keyboard/assistive click (detail 0, no pointer) passes and disarms.
    swallowStrayDisclosureClick();
    const keyboard = click(0, "");
    doc.dispatchEvent(keyboard);
    assertEquals(keyboard.defaultPrevented, false);

    // The touch's compatibility click is swallowed exactly once.
    swallowStrayDisclosureClick();
    const paired = click(1, "touch");
    doc.dispatchEvent(paired);
    assertEquals(paired.defaultPrevented, true);
    const next = click(1, "touch");
    doc.dispatchEvent(next);
    assertEquals(next.defaultPrevented, false);

    // A new gesture takes ownership: its own click is never swallowed.
    swallowStrayDisclosureClick();
    doc.dispatchEvent(new Event("pointerdown"));
    const fresh = click(1, "touch");
    doc.dispatchEvent(fresh);
    assertEquals(fresh.defaultPrevented, false);
  } finally {
    Object.assign(globalThis, { document: original });
  }
});
