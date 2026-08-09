import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_COMPOSER_TOOLBAR,
  normalizeComposerToolbarOrder,
} from "./composerToolbarModel.ts";

const known = (id: string): boolean => id !== "removed-command";

Deno.test("mobile toolbar defaults prioritize editing actions visible without scrolling", () => {
  assertEquals(
    DEFAULT_COMPOSER_TOOLBAR.slice(0, 6),
    ["undo", "redo", "bold", "italic", "code", "link"],
  );
  assertEquals(DEFAULT_COMPOSER_TOOLBAR.includes("attach"), false);
});

Deno.test("the retired default migrates without replacing a curated toolbar", () => {
  const legacy = [
    "undo",
    "redo",
    "heading",
    "bold",
    "italic",
    "strikethrough",
    "highlight",
    "code",
    "link",
    "bulletList",
    "numberedList",
    "checklist",
    "quote",
    "codeBlock",
    "indent",
    "outdent",
    "mention",
    "slash",
    "attach",
  ];
  assertEquals(
    normalizeComposerToolbarOrder(legacy, known),
    DEFAULT_COMPOSER_TOOLBAR,
  );
  assertEquals(
    normalizeComposerToolbarOrder(["italic", "bold", "attach"], known),
    ["italic", "bold", "attach"],
  );
});

Deno.test("stale toolbar ids are removed and malformed storage resets", () => {
  assertEquals(
    normalizeComposerToolbarOrder(
      ["undo", "removed-command", "bold"],
      known,
    ),
    ["undo", "bold"],
  );
  assertEquals(
    normalizeComposerToolbarOrder({}, known),
    DEFAULT_COMPOSER_TOOLBAR,
  );
});
