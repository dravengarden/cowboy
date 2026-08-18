import { assert, assertEquals } from "jsr:@std/assert";
import { drawerProgressAttribute } from "./mobileDrawerMotion.ts";

const drawerSource = await Deno.readTextFile(
  new URL("./mobileSpatialDrawer.ts", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const productShellSource = await Deno.readTextFile(
  new URL("./mobile/shell/MobileProductShell.tsx", import.meta.url),
);
const reviewDrawerSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewDrawerShell.tsx", import.meta.url),
);
const motionSource = await Deno.readTextFile(
  new URL("./mobilePresentationMotion.ts", import.meta.url),
);
const spatialContract = await Deno.readTextFile(
  new URL("../../docs/mobile-spatial-presentation.md", import.meta.url),
);
const webAgents = await Deno.readTextFile(
  new URL("../AGENTS.md", import.meta.url),
);

Deno.test("drawer progress only publishes coarse ownership values", () => {
  assertEquals(drawerProgressAttribute(0), null);
  assertEquals(drawerProgressAttribute(0.02), null);
  assertEquals(drawerProgressAttribute(0.021), "1");
  assertEquals(drawerProgressAttribute(0.347), "1");
  assertEquals(drawerProgressAttribute(1), "1");
});

Deno.test("drawer render stays on a transform-only compositor path", () => {
  assertEquals(drawerSource.includes("drawer.style.opacity"), false);
  assertEquals(drawerSource.includes('willChange = "transform, opacity"'), false);
  assert(drawerSource.includes('layer.style.willChange = "transform"'));
  assertEquals(drawerSource.includes('drawer.style.willChange = "transform"'), false);
  assert(drawerSource.includes("drawerProgressAttribute("));
  assertEquals(drawerSource.includes("drawer.style.opacity"), false);
  assert(drawerSource.includes("dim.style.opacity"));
});

Deno.test("gesture roots flatten overflow tiles without a universal selector", () => {
  assertEquals(
    appSource.includes('&[data-mobile-drawer-moving=\'true\'] *'),
    false,
  );
  assertEquals(
    productShellSource.includes('&[data-mobile-product-moving=\'true\'] *'),
    false,
  );
  assertEquals(
    reviewDrawerSource.includes('&[data-mobile-drawer-moving=\'true\'] *'),
    false,
  );
  assert(appSource.includes("mobilePresentationMovingRootSx"));
  assert(productShellSource.includes("mobilePresentationMovingRootSx"));
  assert(productShellSource.includes("mobileSheetPresentationSx"));
  assert(productShellSource.includes("bindMobileSheetPresentationHold"));
  assert(reviewDrawerSource.includes("mobilePresentationMovingRootSx"));
  assert(motionSource.includes("WebkitOverflowScrolling: \"auto !important\""));
  assert(motionSource.includes("overflow: \"hidden !important\""));
  assert(motionSource.includes("contain: \"paint\""));
  assert(motionSource.includes("pointerEvents: \"none\""));
  assert(motionSource.includes("& [data-mobile-drawer-surface] [data-mobile-overflow-layer]"));
  assertEquals(motionSource.includes("\"& .cm-scroller\""), false);
  assertEquals(motionSource.includes("[data-mobile-drawer-surface] .cm-scroller"), false);
  assertEquals(motionSource.includes("[data-mobile-drawer-surface] .cm-gutters"), false);
  assert(motionSource.includes("& [data-mobile-drawer-surface] [data-key]"));
  assert(motionSource.includes("\"& [data-mobile-overflow-layer]\": mobileOverflowTileFlattenSx"));
  assert(motionSource.includes("\"& [data-mobile-code-layer]\": mobileCodePaintCullSx"));
  assert(motionSource.includes("mobileCodePaintCullSx"));
  assert(motionSource.includes("mobilePeekRestLayerSx"));
  assert(appSource.includes("mobilePeekRestLayerSx"));
  assert(reviewDrawerSource.includes("mobilePeekRestLayerSx"));
  assert(productShellSource.includes("mobilePeekRestLayerSx"));
  assert(motionSource.includes("mobileFrostStripSx"));
  assert(motionSource.includes('attr === "data-mobile-product-moving"'));
  assert(motionSource.includes("& [data-mobile-drawer-surface] .MuiCircularProgress-root"));
  assertEquals(
    motionSource.includes(
      "& .MuiCircularProgress-root, & .MuiSkeleton-root, & [data-mobile-css-animation]",
    ),
    false,
  );
  assertEquals(
    motionSource.includes(
      "& .cm-scroller, & [data-transcript-session], & [data-mobile-overflow-layer]",
    ),
    false,
  );
  assert(motionSource.includes("mobileDrawerRailHitSx"));
  assert(motionSource.includes("[data-mobile-drawer-close='left']"));
  assertEquals(
    motionSource.includes("transform: \"none !important\""),
    false,
  );
  assert(motionSource.includes("& [data-detent-sheet][data-detent-moving]"));
  assert(motionSource.includes("[data-mobile-composer-shell-material]"));
  assert(motionSource.includes("[data-mobile-focus-composer]"));
  assert(motionSource.includes("holdStorePresentation"));
});

Deno.test("jank-free swipe is a core Mobile requirement, not polish", () => {
  assert(spatialContract.includes("core product requirement"));
  assert(spatialContract.includes("Swipe must not jank"));
  assert(webAgents.includes("a swipe that drops frames is a product bug"));
});

Deno.test("product pager marks moving before the first page translate", () => {
  const lock = productShellSource.indexOf("gesture.locked = true");
  const moving = productShellSource.indexOf(
    'shell.setAttribute("data-mobile-product-moving", "true");',
    lock,
  );
  const firstRender = productShellSource.indexOf("pagerOffset(", lock);
  assert(lock !== -1 && moving !== -1 && firstRender !== -1);
  assert(lock < moving && moving < firstRender);
});

Deno.test("settled product pages do not keep a permanent will-change layer", () => {
  assertEquals(productShellSource.includes('willChange: "transform"'), false);
  assertEquals(reviewDrawerSource.includes('willChange: "transform, opacity"'), false);
  assertEquals(reviewDrawerSource.includes('willChange: "transform"'), false);
});
