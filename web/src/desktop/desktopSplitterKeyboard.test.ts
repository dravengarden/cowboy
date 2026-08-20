import { assertEquals } from "jsr:@std/assert";
import {
  adjacentDesktopSplitter,
  preferredDesktopSplitter,
  resolveDesktopResizeSplitter,
  splitterAdjustment,
} from "./desktopSplitterKeyboard.ts";

Deno.test("splitter selection follows the focused pane and Reading surface", () => {
  const agent = ["sessions-prompt", "prompt-conversation"] as const;
  assertEquals(preferredDesktopSplitter(agent, "sessions", "agent"), "sessions-prompt");
  assertEquals(preferredDesktopSplitter(agent, "prompt", "agent"), "prompt-conversation");
  assertEquals(preferredDesktopSplitter(agent, "conversation", "agent"), "prompt-conversation");
  assertEquals(
    preferredDesktopSplitter(["questions-page"], "conversation", "reading"),
    "questions-page",
  );
});

Deno.test("Tab cycles visible splitters in both directions", () => {
  const visible = ["sessions-prompt", "prompt-conversation"] as const;
  assertEquals(
    adjacentDesktopSplitter(visible, "sessions-prompt", 1),
    "prompt-conversation",
  );
  assertEquals(
    adjacentDesktopSplitter(visible, "sessions-prompt", -1),
    "prompt-conversation",
  );
});

Deno.test("width resize keeps a selected bar and otherwise follows the focused pane", () => {
  const visible = ["sessions-prompt", "prompt-conversation"] as const;
  assertEquals(
    resolveDesktopResizeSplitter(visible, "sessions-prompt", "prompt", "agent"),
    "sessions-prompt",
  );
  assertEquals(
    resolveDesktopResizeSplitter(visible, null, "prompt", "agent"),
    "prompt-conversation",
  );
});

Deno.test("splitter adjustment accepts only the typed DOM contract", () => {
  assertEquals(
    splitterAdjustment(new CustomEvent("resize", {
      detail: { splitter: "questions-page", delta: -16 },
    })),
    { splitter: "questions-page", delta: -16 },
  );
  assertEquals(
    splitterAdjustment(new CustomEvent("resize", {
      detail: { splitter: "unknown", delta: -16 },
    })),
    null,
  );
});
