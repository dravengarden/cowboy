import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  DESKTOP_PANES_EXPANDED,
  dragCollapses,
  isPaneZoomed,
  normalizeCollapsedPanes,
  parseCollapsedPanes,
  togglePaneCollapsed,
  toggleZoomPanes,
  withPaneCollapsed,
} from "./desktopLayout.ts";

test("Sessions collapses independently of the work panes", () => {
  const sessions = togglePaneCollapsed(DESKTOP_PANES_EXPANDED, "sessions");
  assertEquals(sessions, { sessions: true, prompt: false, conversation: false });
  assertEquals(togglePaneCollapsed(sessions, "sessions"), DESKTOP_PANES_EXPANDED);
});

test("collapsing the last visible work pane swaps it with its sibling", () => {
  const promptHidden = togglePaneCollapsed(DESKTOP_PANES_EXPANDED, "prompt");
  assertEquals(promptHidden, { sessions: false, prompt: true, conversation: false });
  assertEquals(togglePaneCollapsed(promptHidden, "conversation"), {
    sessions: false,
    prompt: false,
    conversation: true,
  });
  const conversationHidden = togglePaneCollapsed(DESKTOP_PANES_EXPANDED, "conversation");
  assertEquals(togglePaneCollapsed(conversationHidden, "prompt"), {
    sessions: false,
    prompt: true,
    conversation: false,
  });
});

test("an unchanged collapse request keeps the same snapshot", () => {
  assertEquals(
    withPaneCollapsed(DESKTOP_PANES_EXPANDED, "prompt", false) === DESKTOP_PANES_EXPANDED,
    true,
  );
});

test("stored layouts never hide both work panes", () => {
  assertEquals(
    normalizeCollapsedPanes({ sessions: true, prompt: true, conversation: true }),
    { sessions: true, prompt: true, conversation: false },
  );
  assertEquals(
    parseCollapsedPanes('{"sessions":true,"prompt":true,"conversation":true}'),
    { sessions: true, prompt: true, conversation: false },
  );
  assertEquals(parseCollapsedPanes("not json"), DESKTOP_PANES_EXPANDED);
  assertEquals(parseCollapsedPanes("null"), DESKTOP_PANES_EXPANDED);
  assertEquals(parseCollapsedPanes('{"sessions":"yes"}'), DESKTOP_PANES_EXPANDED);
});

test("drag-to-collapse needs a deliberate overshoot past the minimum", () => {
  assertEquals(dragCollapses(240, 240), false);
  assertEquals(dragCollapses(150, 240), false);
  assertEquals(dragCollapses(143, 240), true);
});

test("the collapsed-Sessions split ratio stays within usable bounds", async () => {
  const { clampPromptRatio } = await import("./desktopLayout.ts");
  assertEquals(clampPromptRatio(0.5), 0.5);
  assertEquals(clampPromptRatio(0.1), 0.25);
  assertEquals(clampPromptRatio(0.95), 0.75);
});

test("zoom focuses the active work pane and restores the previous layout", () => {
  const start = { sessions: false, prompt: false, conversation: false };
  const zoomedPrompt = toggleZoomPanes(start, null, "prompt", true, false);
  assertEquals(zoomedPrompt, {
    next: { sessions: true, prompt: false, conversation: true },
    saved: start,
    target: "prompt",
  });
  assertEquals(isPaneZoomed(zoomedPrompt.next, true, false), true);
  const restored = toggleZoomPanes(
    zoomedPrompt.next,
    zoomedPrompt.saved,
    "prompt",
    true,
    false,
  );
  assertEquals(restored, { next: start, saved: null, target: "prompt" });

  // Preserves custom pre-zoom layout (e.g., Sessions already collapsed).
  const noSessions = { sessions: true, prompt: false, conversation: false };
  const zoomedConv = toggleZoomPanes(noSessions, null, "conversation", true, false);
  assertEquals(zoomedConv.next, { sessions: true, prompt: true, conversation: false });
  assertEquals(
    toggleZoomPanes(zoomedConv.next, zoomedConv.saved, "conversation", true, false).next,
    noSessions,
  );

  // Without a saved snapshot (e.g., after reload), unzoom expands all panes.
  assertEquals(
    toggleZoomPanes({ sessions: true, prompt: true, conversation: false }, null, "conversation", true, false).next,
    DESKTOP_PANES_EXPANDED,
  );

  // Draft page zooms by collapsing Sessions and restores Sessions on second press.
  const draftZoom = toggleZoomPanes(start, null, "prompt", false, false);
  assertEquals(draftZoom.next, { sessions: true, prompt: false, conversation: false });
  assertEquals(isPaneZoomed(draftZoom.next, false, false), true);
  assertEquals(
    toggleZoomPanes(draftZoom.next, draftZoom.saved, "prompt", false, false).next,
    start,
  );

  // Compact width toggles only the sibling work pane.
  const compactZoom = toggleZoomPanes(start, null, "conversation", true, true);
  assertEquals(compactZoom.next, { sessions: false, prompt: true, conversation: false });
  assertEquals(isPaneZoomed(compactZoom.next, true, true), true);
  assertEquals(
    toggleZoomPanes(compactZoom.next, compactZoom.saved, "conversation", true, true).next,
    start,
  );
});
