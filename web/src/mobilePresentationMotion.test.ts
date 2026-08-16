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
  assert(motionSource.includes("WebkitOverflowScrolling: \"auto\""));
  assert(motionSource.includes("contain: \"paint\""));
  assert(motionSource.includes("pointerEvents: \"none\""));
  assert(motionSource.includes("& [data-mobile-drawer-surface] [data-mobile-overflow-layer]"));
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
  assert(motionSource.includes("[data-mobile-focus-composer]"));
  assert(motionSource.includes("holdStorePresentation"));
});

Deno.test("settled product pages do not keep a permanent will-change layer", () => {
  assertEquals(productShellSource.includes('willChange: "transform"'), false);
  assertEquals(reviewDrawerSource.includes('willChange: "transform, opacity"'), false);
  assertEquals(reviewDrawerSource.includes('willChange: "transform"'), false);
});
