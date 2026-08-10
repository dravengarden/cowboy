import { assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const rowStart = appSource.indexOf("const ReliableListItemButton");
const rowEnd = appSource.indexOf("function SessionList", rowStart);
const rowSource = appSource.slice(rowStart, rowEnd);

Deno.test("touch session rows do not retain synthetic hover or focus paint", () => {
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
