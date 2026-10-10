import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const source = await readFile(
  new URL("./SegmentedTabs.tsx", import.meta.url), "utf8",
);

// Every view switcher renders through SegmentedTabs; these pin the iOS lessons
// each hand-rolled copy used to carry (appSettings / reviewCommitLayout tests).
test("segmented tabs keep the selected pill after an iOS touch", () => {
  assert(source.includes("disableRipple"));
  assert(
    source.includes('event.currentTarget.dataset.touchActivated = "true"'),
  );
  assert(source.includes("event.currentTarget.blur()"));
  assert(source.includes("&&[data-selected='true']"));
  assert(source.includes("COARSE_POINTER_ROOT_CLASS"));
  assert(
    source.includes(
      "[data-touch-activated='true'][data-selected='false']:hover",
    ),
  );
  assert(source.includes('bgcolor: "action.selected"'));
});

test("segmented tabs stay paint-only inside the Mobile peek", () => {
  // No sliding thumb: a nested transform or shadow inside the swipe layer
  // reassembles tiles on every frame (docs/mobile-spatial-presentation.md).
  assertEquals(source.includes("translateX"), false);
  assertEquals(source.includes("boxShadow"), false);
  assertEquals(source.includes("backdropFilter"), false);
  assert(source.includes('transform: "none"'));
});

test("segmented tabs expose tab or toggle semantics", () => {
  assert(source.includes('role={tabs ? "tablist" : "group"}'));
  assert(source.includes("aria-selected={tabs ? selected : undefined}"));
  assert(source.includes("aria-pressed={tabs ? undefined : selected}"));
  for (const key of ["ArrowRight", "ArrowLeft", "Home", "End"]) {
    assert(source.includes(`"${key}"`));
  }
});

test("Desktop tablists add Vim keys through the shared key intent", () => {
  assert(source.includes("desktopKeyIntent(event.nativeEvent)"));
  assert(source.includes('if (intent.owner === "ime") return;'));
  assert(source.includes('intent.key === "h"'));
  assert(source.includes('intent.key === "l"'));
  // Roving keeps focus on the tablist; only activation commits to a panel.
  assert(source.includes('onChange(next.value, "roving")'));
  assert(source.includes('onChange(option.value, "activate")'));
});
