import { assert } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("mobile drawer translates an inner layer that owns the footer", () => {
  assert(appSource.includes("mobileLayerRef.current ?? columnRef.current"));
  assert(
    appSource.includes(
      'data-mobile-drawer-surface={mobile ? "true" : undefined}',
    ),
  );
  assert(appSource.includes('data-mobile-session-footer="true"'));
  assert(appSource.includes("isolation: \"isolate\""));
  assert(appSource.includes("inset: 0"));
  assert(appSource.includes("calc(-1 * var(--navbar-h, 0px))"));
});
