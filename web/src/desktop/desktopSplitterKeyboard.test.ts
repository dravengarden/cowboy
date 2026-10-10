import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  adjacentDesktopSplitter,
  preferredDesktopSplitter,
  resolveDesktopResizeSplitter,
  splitterAdjustment,
} from "./desktopSplitterKeyboard.ts";

test("splitter selection follows the focused pane", () => {
  const agent = ["sessions-prompt", "prompt-conversation"] as const;
  assertEquals(preferredDesktopSplitter(agent, "sessions"), "sessions-prompt");
  assertEquals(preferredDesktopSplitter(agent, "prompt"), "prompt-conversation");
  assertEquals(preferredDesktopSplitter(agent, "conversation"), "prompt-conversation");
});

test("Tab cycles visible splitters in both directions", () => {
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

test("width resize keeps a selected bar and otherwise follows the focused pane", () => {
  const visible = ["sessions-prompt", "prompt-conversation"] as const;
  assertEquals(
    resolveDesktopResizeSplitter(visible, "sessions-prompt", "prompt"),
    "sessions-prompt",
  );
  assertEquals(
    resolveDesktopResizeSplitter(visible, null, "prompt"),
    "prompt-conversation",
  );
});

test("splitter adjustment accepts only the typed DOM contract", () => {
  assertEquals(
    splitterAdjustment(new CustomEvent("resize", {
      detail: { splitter: "prompt-conversation", delta: -16 },
    })),
    { splitter: "prompt-conversation", delta: -16 },
  );
  assertEquals(
    splitterAdjustment(new CustomEvent("resize", {
      detail: { splitter: "unknown", delta: -16 },
    })),
    null,
  );
});
