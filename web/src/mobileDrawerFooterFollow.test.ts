import { assert, assertEquals } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const reviewDrawerSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewDrawerShell.tsx", import.meta.url),
);

Deno.test("mobile drawer translates an in-flow page that owns the footer", () => {
  assert(appSource.includes("mobilePageRef.current ?? mobileLayerRef.current"));
  assert(
    appSource.includes(
      'data-mobile-drawer-surface={mobile ? "true" : undefined}',
    ),
  );
  assert(appSource.includes('data-mobile-session-footer={mobile ? "true" : undefined}'));
  assert(appSource.includes('data-mobile-drawer-follow={mobile ? "true" : undefined}'));
  assert(appSource.includes("getFollowers: () => ["));
  assert(appSource.includes("mobileComposerFollowRef.current"));
  assert(appSource.includes("mobileNavFollowRef.current"));
  assert(appSource.includes('isolation: "isolate"'));
  // A transformed position:absolute;inset:0 page lets iOS pin the footer.
  assertEquals(appSource.includes("calc(-1 * var(--navbar-h, 0px))"), false);
  assert(appSource.includes("Transcript-only sliding page"));
  assert(appSource.includes('bgcolor: mobile ? "transparent" : "background.default"'));
  assert(appSource.includes('bottomInset={mobile'));
  assert(appSource.includes('{!mobile && ('));
  assert(reviewDrawerSource.includes("In-flow fill. A transformed position:absolute;inset:0"));
  assertEquals(
    /data-mobile-drawer-surface="true"[\s\S]{0,220}inset: 0/.test(
      reviewDrawerSource,
    ),
    false,
  );
});
