import { assert, assertEquals } from "jsr:@std/assert";

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

Deno.test("mobile drawer keeps clipping and shadows off the heavy surface", () => {
  assertEquals(drawerSource.includes("surface.style.borderRadius"), false);
  assertEquals(drawerSource.includes("surface.style.boxShadow"), false);
  assert(drawerSource.includes("drawerMask.style.boxShadow"));
  assert(drawerSource.includes('surface.style.willChange = "transform"'));
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
    'boxShadow: drawerOpen\n                            ? "18px 0 42px rgba(0,0,0,0.16)"',
  ));
  assert(reviewDrawerSource.includes(
    'data-mobile-drawer-presented={open ? "true" : undefined}',
  ));
  assert(reviewDrawerSource.includes(
    'boxShadow: open ? "-18px 0 42px rgba(0,0,0,0.16)" : "none"',
  ));
  assert(productShellSource.includes(
    '"[data-mobile-drawer-presented=\'true\']',
  ));
});
