import { assertEquals } from "jsr:@std/assert";
import { nextComposerStackExpanded } from "./composerStackAccordion.ts";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const planSource = await Deno.readTextFile(
  new URL("./PlanDock.tsx", import.meta.url),
);
const pendingSource = await Deno.readTextFile(
  new URL("./pendingPanelState.ts", import.meta.url),
);

Deno.test("composer stack disclosure is exclusive and can collapse all", () => {
  assertEquals(nextComposerStackExpanded(null, "draft"), "draft");
  assertEquals(nextComposerStackExpanded("draft", "draft"), null);
  assertEquals(nextComposerStackExpanded("draft", "queued"), "queued");
  assertEquals(nextComposerStackExpanded("queued", "plan"), "plan");
  assertEquals(nextComposerStackExpanded("plan", "plan"), null);
});

Deno.test("plan queue and draft share the exclusive stack accordion", () => {
  assertEquals(planSource.includes("toggleComposerStackPanel(\"plan\")"), true);
  assertEquals(composerSource.includes("toggleComposerStackPanel(kind)"), true);
  assertEquals(
    pendingSource.includes("expandComposerStackPanel(arrival.kind)"),
    true,
  );
  assertEquals(composerSource.includes("unbounded"), false);
  assertEquals(
    composerSource.includes('data-mobile-pending-scrollport={!desktop ? "true" : undefined}'),
    true,
  );
  assertEquals(
    composerSource.includes('maxHeight: "30vh"'),
    true,
  );
  assertEquals(
    planSource.includes('maxHeight: desktop ? 176 : "30vh"'),
    true,
  );
  assertEquals(composerSource.includes("data-mobile-pending-scrollport"), true);
  assertEquals(
    composerSource.includes("data-mobile-pending-scrollport\n"),
    false,
  );
});
