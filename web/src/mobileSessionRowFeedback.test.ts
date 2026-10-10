import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const rowSource = await readFile(new URL("./ReliableListItemButton.tsx", import.meta.url), "utf8");

test("session rows show compact project context", () => {
  assertEquals(
    /secondary=\{[\s\S]*?\{sessionListProjectLabel\(s\)\}/.test(appSource),
    true,
  );
});

test("touch session rows do not retain synthetic hover or focus paint", () => {
  assertEquals(
    rowSource.includes('event.currentTarget.dataset.touchActivated = "true"'),
    true,
  );
  assertEquals(
    rowSource.includes(
      "&[data-touch-activated='true']:not(.Mui-selected):hover, &[data-touch-activated='true'].Mui-focusVisible:not(.Mui-selected)",
    ),
    true,
  );
  assertEquals(
    rowSource.includes(
      "&[data-touch-activated='true'].Mui-selected:hover, &[data-touch-activated='true'].Mui-selected.Mui-focusVisible",
    ),
    true,
  );
  assertEquals(
    rowSource.match(/delete event\.currentTarget\.dataset\.touchActivated/g)
      ?.length,
    3,
  );
});

test("session rows mark grip touches before propagation is stopped", () => {
  const capture = rowSource.indexOf("onPointerDownCapture={(event)");
  const bubble = rowSource.indexOf("onPointerDown={(event)");
  assertEquals(capture >= 0, true);
  assertEquals(bubble > capture, true);
  assertEquals(
    rowSource.slice(capture, bubble).includes(
      'event.currentTarget.dataset.touchActivated = "true"',
    ),
    true,
  );
});
