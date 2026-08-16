import { assertEquals } from "jsr:@std/assert";

const themeSource = await Deno.readTextFile(
  new URL("./theme.ts", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("touch icon buttons release synthetic hover and focus paint", () => {
  assertEquals(
    themeSource.includes('"@media (hover: none), (pointer: coarse)"'),
    true,
  );
  assertEquals(
    themeSource.includes(
      '"&:not(.Mui-selected):hover, &:not(.Mui-selected).Mui-focusVisible"',
    ),
    true,
  );
  assertEquals(
    themeSource.includes('"&:not(.Mui-selected):active"'),
    true,
  );
  assertEquals(
    appSource.includes(
      "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible",
    ),
    true,
  );
});
