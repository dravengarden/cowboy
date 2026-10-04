import { assertEquals } from "jsr:@std/assert";
import {
  DESKTOP_PANES_EXPANDED,
  dragCollapses,
  normalizeCollapsedPanes,
  parseCollapsedPanes,
  togglePaneCollapsed,
  withPaneCollapsed,
} from "./desktopLayout.ts";

Deno.test("Sessions collapses independently of the work panes", () => {
  const sessions = togglePaneCollapsed(DESKTOP_PANES_EXPANDED, "sessions");
  assertEquals(sessions, { sessions: true, prompt: false, conversation: false });
  assertEquals(togglePaneCollapsed(sessions, "sessions"), DESKTOP_PANES_EXPANDED);
});

Deno.test("collapsing the last visible work pane swaps it with its sibling", () => {
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

Deno.test("an unchanged collapse request keeps the same snapshot", () => {
  assertEquals(
    withPaneCollapsed(DESKTOP_PANES_EXPANDED, "prompt", false) === DESKTOP_PANES_EXPANDED,
    true,
  );
});

Deno.test("stored layouts never hide both work panes", () => {
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

Deno.test("drag-to-collapse needs a deliberate overshoot past the minimum", () => {
  assertEquals(dragCollapses(240, 240), false);
  assertEquals(dragCollapses(150, 240), false);
  assertEquals(dragCollapses(143, 240), true);
});

Deno.test("the collapsed-Sessions split ratio stays within usable bounds", async () => {
  const { clampPromptRatio } = await import("./desktopLayout.ts");
  assertEquals(clampPromptRatio(0.5), 0.5);
  assertEquals(clampPromptRatio(0.1), 0.25);
  assertEquals(clampPromptRatio(0.95), 0.75);
});
