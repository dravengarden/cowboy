import { assertEquals } from "jsr:@std/assert";
import {
  COARSE_POINTER_ROOT_CLASS,
  prefersCoarsePointer,
  syncCoarsePointerRootClass,
} from "./platform.ts";

const themeSource = await Deno.readTextFile(
  new URL("./theme.ts", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const indexSource = await Deno.readTextFile(
  new URL("../index.html", import.meta.url),
);

Deno.test("touch icon buttons release synthetic hover and focus paint", () => {
  assertEquals(
    themeSource.includes(
      '"@media (hover: none), (pointer: coarse), (any-pointer: coarse)"',
    ),
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
  assertEquals(themeSource.includes('"--IconButton-hoverBg"'), true);
  assertEquals(
    themeSource.includes("&.MuiIconButton-root.MuiIconButton-colorPrimary"),
    true,
  );
  assertEquals(themeSource.includes("disableFocusRipple: prefersCoarsePointer()"), true);
  assertEquals(
    themeSource.includes(`html.\${COARSE_POINTER_ROOT_CLASS} &`),
    true,
  );
  assertEquals(
    appSource.includes(
      "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible",
    ),
    true,
  );
  assertEquals(themeSource.includes("MuiButtonBase:"), true);
  assertEquals(themeSource.includes("disableRipple: prefersCoarsePointer()"), true);
  assertEquals(themeSource.includes("disableTouchRipple: prefersCoarsePointer()"), true);
  assertEquals(themeSource.includes("WebkitTapHighlightColor: \"transparent\""), true);
});

Deno.test("session sheet trigger releases synthetic hover like other navbar icons", () => {
  assertEquals(
    composerSource.includes(
      "&[data-touch-activated='true']:hover, &[data-touch-activated='true'].Mui-focusVisible",
    ),
    true,
  );
  assertEquals(composerSource.includes("if (touchInput) event.currentTarget.blur()"), true);
  assertEquals(indexSource.includes("-webkit-tap-highlight-color: transparent"), true);
});

Deno.test("coarse pointer includes a finger even when iOS later reports hover", () => {
  assertEquals(
    prefersCoarsePointer({ maxTouchPoints: 0, anyPointerCoarse: false }),
    false,
  );
  assertEquals(
    prefersCoarsePointer({ maxTouchPoints: 5, anyPointerCoarse: false }),
    true,
  );
  assertEquals(
    prefersCoarsePointer({ maxTouchPoints: 0, anyPointerCoarse: true }),
    true,
  );
});

Deno.test("coarse pointer class is pinned on the document root", () => {
  const classes = new Set<string>();
  const coarse = syncCoarsePointerRootClass({
    documentElement: {
      classList: {
        toggle(name, force): boolean {
          if (force) classes.add(name);
          else classes.delete(name);
          return force;
        },
      },
    },
  });
  assertEquals(coarse, prefersCoarsePointer());
  assertEquals(classes.has(COARSE_POINTER_ROOT_CLASS), coarse);
});
