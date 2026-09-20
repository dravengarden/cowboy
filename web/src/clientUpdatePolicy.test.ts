import { assert, assertEquals } from "jsr:@std/assert";
import { tickUpdateCountdown, updateAllowed } from "../../components/app-shell/update-policy.ts";

const bannerSource = await Deno.readTextFile(
  new URL("../../components/app-shell/connection-banner.tsx", import.meta.url),
);
const mobileSource = await Deno.readTextFile(
  new URL("./mobile/MobileConnectionBanner.tsx", import.meta.url),
);

Deno.test("busy work outranks every other reason to update", () => {
  const busy = { idle: false, visible: true, visibleForMs: 10 * 60_000 };
  assertEquals(updateAllowed(busy, 0), false);
  assertEquals(updateAllowed(busy, 60_000), false);
});

Deno.test("a surface without a dwell applies as soon as the user is idle", () => {
  assertEquals(
    updateAllowed({ idle: true, visible: false, visibleForMs: 0 }, 0),
    true,
  );
});

Deno.test("a dwelling surface waits out its foreground minute", () => {
  const dwell = 60_000;
  assertEquals(
    updateAllowed({ idle: true, visible: true, visibleForMs: 59_999 }, dwell),
    false,
  );
  assertEquals(
    updateAllowed({ idle: true, visible: true, visibleForMs: 60_000 }, dwell),
    true,
  );
  // A backgrounded page cannot bank dwell it did not spend in front of anyone.
  assertEquals(
    updateAllowed(
      { idle: true, visible: false, visibleForMs: 10 * 60_000 },
      dwell,
    ),
    false,
  );
});

Deno.test("the countdown rewinds whole rather than freezing mid-count", () => {
  assertEquals(tickUpdateCountdown({ secs: 3, held: false }, true, 3), {
    secs: 2,
    held: false,
  });
  assertEquals(tickUpdateCountdown({ secs: 1, held: false }, true, 3), {
    secs: 0,
    held: false,
  });
  assertEquals(tickUpdateCountdown({ secs: 1, held: false }, false, 3), {
    secs: 3,
    held: true,
  });
});

Deno.test("a held countdown keeps re-arming its check", () => {
  // Rewinding to the same second leaves the effect's other dependencies
  // unchanged, so the timer is only rescheduled because `recheck` moves. Losing
  // it strands a busy page on the old build until it is reloaded by hand.
  const hook = bannerSource.slice(
    bannerSource.indexOf("export function useAutoUpdate"),
    bannerSource.indexOf("// MUI palette per banner kind"),
  );
  assert(hook.includes("setRecheck((value) => value + 1);"));
  assert(hook.includes("    recheck,\n"));
});

Deno.test("both surfaces apply the update without a control to press", () => {
  assert(bannerSource.includes("useAutoUpdate(store, {"));
  // Desktop keeps its dwell-free policy; the phone earns a foreground minute.
  assert(!bannerSource.includes("minVisibleMs: "));
  assert(mobileSource.includes("minVisibleMs: MOBILE_UPDATE_DWELL_MS"));
  assert(mobileSource.includes("const MOBILE_UPDATE_DWELL_MS = 60_000;"));
  assertEquals(mobileSource.includes("<Button"), false);
  assertEquals(mobileSource.includes("onClick"), false);
  assertEquals(mobileSource.includes("store.applyUpdate()"), false);
});
