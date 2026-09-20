import { assert, assertEquals } from "jsr:@std/assert";
import {
  tickUpdateCountdown,
  updateAllowed,
  updateReloadsNow,
} from "../../components/app-shell/update-policy.ts";
import {
  updateFillShare,
  updateFillSx,
  updatePercentLabel,
} from "../../components/app-shell/update-presentation.ts";

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

Deno.test("nothing replaces a running build before its replacement is here", () => {
  // Not even the press: the whole point of the control is that taking the
  // update is local and instant, which is only true once the bits are cached.
  assertEquals(
    updateReloadsNow({ downloaded: false, requested: true, countedDown: true }),
    false,
  );
  assertEquals(
    updateReloadsNow({ downloaded: false, requested: false, countedDown: false }),
    false,
  );
});

Deno.test("a press outranks the idle gate the countdown answers to", () => {
  // The gate protects someone who did not ask. This someone is looking at the
  // control they just pressed, so they do not wait out a countdown as well.
  assertEquals(
    updateReloadsNow({ downloaded: true, requested: true, countedDown: false }),
    true,
  );
  // And the automatic road still arrives for everyone who never presses.
  assertEquals(
    updateReloadsNow({ downloaded: true, requested: false, countedDown: true }),
    true,
  );
  assertEquals(
    updateReloadsNow({ downloaded: true, requested: false, countedDown: false }),
    false,
  );
});

Deno.test("a held countdown keeps re-arming its check", () => {
  // Rewinding to the same second leaves the effect's other dependencies
  // unchanged, so the timer is only rescheduled because `recheck` moves. Losing
  // it strands a busy page on the old build until it is reloaded by hand.
  const hook = bannerSource.slice(
    bannerSource.indexOf("export function useAutoUpdate"),
    bannerSource.indexOf("// Liveview's exact English labels"),
  );
  assert(hook.includes("setRecheck((value) => value + 1);"));
  assert(hook.includes("    recheck,\n"));
});

Deno.test("the download starts on detection, not on the idle gate", () => {
  // What interrupts someone is the reload, never the download. Gating the
  // fetch on idleness would put the wait back where the user can feel it and
  // leave the control unable to promise an instant swap.
  const effect = bannerSource.slice(
    bannerSource.indexOf("void store.downloadUpdate("),
    bannerSource.indexOf("}, [pending, attempt, store]);"),
  );
  assert(effect.length > 0);
  assert(!effect.includes("canApplyUpdate"));
  assert(!effect.includes("updateAllowed"));
});

Deno.test("both surfaces update themselves, and both offer to be pressed", () => {
  assert(bannerSource.includes("useAutoUpdate(store, {"));
  // Desktop keeps its dwell-free policy; the phone earns a foreground minute.
  assert(!bannerSource.includes("minVisibleMs: "));
  assert(mobileSource.includes("minVisibleMs: MOBILE_UPDATE_DWELL_MS"));
  assert(mobileSource.includes("const MOBILE_UPDATE_DWELL_MS = 60_000;"));
  assert(mobileSource.includes("onClick={update.requestUpdate}"));
  assert(bannerSource.includes("onClick={update.requestUpdate}"));
  // The control is never withheld while the bits are still coming: a press
  // then means "as soon as it lands", and a disabled bar on a phone reads as
  // broken. Only the swap itself, which cannot be taken back, stops answering.
  assert(mobileSource.includes('disabled={update.phase === "reloading"}'));
  assert(bannerSource.includes('disabled={update.phase === "reloading"}'));
});

Deno.test("the page asks for progress rather than assuming it", () => {
  // The worker on the other end may predate progress entirely, and the worker
  // this page talks to has the previous build as its other caller. Opting in by
  // flag keeps both directions of that transition working.
  assert(bannerSource.includes(
    `controller.postMessage({ type: "cowboy.refresh-shell", progress: true }, [channel.port2])`,
  ));
});

Deno.test("a stalled download keeps the ground it took", () => {
  assertEquals(updateFillShare("downloading", 0.4), 0.4);
  assertEquals(updateFillShare("failed", 0.4), 0.4);
  // Ready fills whole even where there was no progress to report: a page no
  // service worker controls has nothing to pre-fetch, and an empty bar over a
  // pressable control would misread as unfinished.
  assertEquals(updateFillShare("ready", undefined), 1);
  assertEquals(updateFillShare("reloading", undefined), 1);
  assertEquals(updateFillShare("downloading", undefined), 0);
});

Deno.test("the bar never reads 100% before the bits are here", () => {
  assertEquals(updatePercentLabel(0.999, false), "99%");
  assertEquals(updatePercentLabel(1, false), "99%");
  assertEquals(updatePercentLabel(1, true), "100%");
  assertEquals(updatePercentLabel(0.62, false), "62%");
  assertEquals(updatePercentLabel(undefined, false), undefined);
});

Deno.test("the fill is one paint-only background, and it never sweeps a lie", () => {
  const streamed = updateFillSx("#0288d1", "#01579b", 0.5, true);
  assertEquals(streamed.backgroundSize, "50% 100%");
  assertEquals(streamed.backgroundColor, "#01579b");
  assert(streamed.transition.startsWith("background-size 400ms"));
  // A build that was already cached resolves at once; sweeping it through
  // progress it never spent would be theatre in an offline-first app.
  assertEquals(updateFillSx("#0288d1", "#01579b", 1, false).transition, "none");
  // No transform, no shadow: the phone's moving chrome forbids both.
  const keys = Object.keys(updateFillSx("#0288d1", "#01579b", 1, false));
  assertEquals(keys.some((key) => key.includes("transform") || key.includes("Shadow")), false);
});
