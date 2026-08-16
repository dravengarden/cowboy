import { assert, assertEquals } from "jsr:@std/assert";
import { mobileSpatialDrawerShadow } from "./mobileDrawerDepth.ts";

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

Deno.test("dispose keeps seam shadow while a drawer is still translated", () => {
  assert(drawerSource.includes("shouldKeepDrawerDepth(getOpen(), currentOffset)"));
  assert(drawerSource.includes("data-mobile-drawer-progress"));
});

Deno.test("mobile drawer keeps clipping and shadows off the heavy surface", () => {
  assertEquals(drawerSource.includes("surface.style.boxShadow"), false);
  assertEquals(drawerSource.includes("surface.style.borderRadius"), false);
  assertEquals(drawerSource.includes("applyCardChrome"), false);
  assert(drawerSource.includes("drawerMask.style.boxShadow"));
  assert(drawerSource.includes("mobileDrawerCardVisual"));
  assert(drawerSource.includes('surface.style.willChange = "transform"'));
  assert(drawerSource.includes("--mobile-drawer-width"));
  assertEquals(drawerSource.includes("scheduleRender"), false);
  assertEquals(drawerSource.includes("drawerParallax"), false);
  assertEquals(drawerSource.includes("scale("), false);
  assert(drawerSource.includes("stepDrawerSpring"));
  assert(drawerSource.includes("MOBILE_DRAWER_PREPARE_PX"));
  assert(drawerSource.includes("gesture.prepared = true"));
  const widthAt = drawerSource.indexOf("publishDrawerWidth(width)");
  const firstTranslate = drawerSource.indexOf("render(offset)");
  assert(widthAt >= 0 && firstTranslate > widthAt);
});

Deno.test("drawer join is a hard paper|page edge", () => {
  assertEquals(mobileSpatialDrawerShadow("left"), "none");
  assertEquals(mobileSpatialDrawerShadow("right"), "none");
});

Deno.test("Agent drawer publishes pager ownership synchronously", () => {
  const bindingStart = appSource.indexOf("const binding = bindMobileSpatialDrawer({");
  const bindingEnd = appSource.indexOf("holdPresentation: holdStorePresentation", bindingStart);
  assert(bindingStart >= 0 && bindingEnd > bindingStart);
  const binding = appSource.slice(bindingStart, bindingEnd);
  const refUpdate = binding.indexOf("drawerOpenRef.current = open");
  const parentUpdate = binding.indexOf("onMobileDrawerOpenChange?.(open)");
  const reactUpdate = binding.indexOf("setDrawerOpen(open)");
  assert(refUpdate >= 0);
  assert(parentUpdate > refUpdate);
  assert(reactUpdate > parentUpdate);
  assertEquals(
    appSource.includes("onMobileDrawerOpenChange?.(drawerOpen)"),
    false,
  );
});

Deno.test("settled drawers retain declarative depth and pager ownership", () => {
  assert(appSource.includes(
    'data-mobile-drawer-presented={mobile && drawerOpen ? "true" : undefined}',
  ));
  assert(appSource.includes(
    'boxShadow: drawerOpen\n                            ? mobileSpatialDrawerShadow("left")',
  ));
  assert(reviewDrawerSource.includes(
    'data-mobile-drawer-presented={open ? "true" : undefined}',
  ));
  assert(reviewDrawerSource.includes(
    'boxShadow: open ? mobileSpatialDrawerShadow("right") : "none"',
  ));
  assert(productShellSource.includes(
    '"[data-mobile-drawer-presented=\'true\']',
  ));
  assert(productShellSource.includes("translatedSurfaceOwnsPagerGesture"));
  assert(productShellSource.includes("drawerProgressOwnsPagerGesture"));
  assert(appSource.includes("data-mobile-drawer-surface={mobile ? \"true\" : undefined}"));
  assert(reviewDrawerSource.includes('data-mobile-drawer-surface="true"'));
  assert(appSource.includes("width: 28"));
  assert(reviewDrawerSource.includes("width: 28"));
  assert(appSource.includes(
    'width: "var(--mobile-drawer-width, min(84%, 360px))"',
  ));
  assert(reviewDrawerSource.includes(
    'pl: "calc(100% - var(--mobile-drawer-width, min(84%, 360px)))"',
  ));
  assert(appSource.includes('bgcolor: "background.default"'));
  assert(reviewDrawerSource.includes('bgcolor: "background.default"'));
});
