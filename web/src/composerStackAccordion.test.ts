import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { nextComposerStackExpanded } from "./composerStackAccordion.ts";

const composerSource = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const planSource = await readFile(
  new URL("./PlanDock.tsx", import.meta.url), "utf8",
);
const pendingSource = await readFile(
  new URL("./pendingPanelState.ts", import.meta.url), "utf8",
);

test("composer stack disclosure is exclusive and can collapse all", () => {
  assertEquals(nextComposerStackExpanded(null, "draft"), "draft");
  assertEquals(nextComposerStackExpanded("draft", "draft"), null);
  assertEquals(nextComposerStackExpanded("draft", "queued"), "queued");
  assertEquals(nextComposerStackExpanded("queued", "plan"), "plan");
  assertEquals(nextComposerStackExpanded("plan", "plan"), null);
});

test("plan queue and draft share the exclusive stack accordion", () => {
  assertEquals(planSource.includes("toggleComposerStackPanel(\"plan\")"), true);
  assertEquals(composerSource.includes("toggleComposerStackPanel(kind)"), true);
  assertEquals(
    pendingSource.includes("expandComposerStackPanel(arrival.kind)"),
    true,
  );
  assertEquals(composerSource.includes("unbounded"), false);
  assertEquals(
    composerSource.includes(
      "data-mobile-pending-scrollport={!desktop && !visuallyCollapsed",
    ),
    true,
  );
  assertEquals(
    composerSource.includes('maxHeight: "30vh"'),
    true,
  );
  assertEquals(
    composerSource.includes(
      'overflowY: visuallyCollapsed ? "hidden" : "auto"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'WebkitOverflowScrolling: visuallyCollapsed ? "auto" : "touch"',
    ),
    true,
  );
  assertEquals(
    planSource.includes('maxHeight: desktop ? 176 : "30vh"'),
    true,
  );
  assertEquals(
    planSource.includes('overflowY: expanded ? "auto" : "hidden"'),
    true,
  );
  assertEquals(composerSource.includes("data-mobile-pending-scrollport"), true);
  assertEquals(
    composerSource.includes("data-mobile-pending-scrollport\n"),
    false,
  );
});
