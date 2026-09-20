import { assertEquals } from "jsr:@std/assert";
import {
  cowboyVersionFromServiceWorkerSource,
  fetchReadyCowboyVersion,
  mobileUpdateAnnouncement,
  mobileUpdateBanner,
} from "./mobileUpdateVersion.ts";

Deno.test("the update bar names the ready service-worker version", () => {
  assertEquals(
    cowboyVersionFromServiceWorkerSource('const VERSION = "cowboy-v1352";'),
    "cowboy-v1352",
  );
  assertEquals(
    cowboyVersionFromServiceWorkerSource("const VERSION = 'cowboy-v12';"),
    "cowboy-v12",
  );
  assertEquals(
    cowboyVersionFromServiceWorkerSource("const ASSET_CACHE"),
    undefined,
  );
});

Deno.test("the phone bar narrates a download it does not offer to hurry", () => {
  // A download nobody asked for shows as a hairline, so this copy only
  // appears when there is something to say; either way it names no action,
  // because there is nothing useful to press while bits are in flight.
  const quiet = mobileUpdateBanner("cowboy-v1352", {
    kind: "downloading",
    progress: 0.62,
    requested: false,
  });
  assertEquals(quiet, { text: "Cowboy cowboy-v1352 · 62%" });
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "downloading", requested: false }).text,
    "Cowboy cowboy-v1352 · downloading…",
  );
  assertEquals(
    mobileUpdateBanner(undefined, { kind: "downloading", progress: 0.2, requested: false }).text,
    "New Cowboy version · 20%",
  );
  // Never 100% before the bits are here, however the fraction rounds.
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "downloading", progress: 0.999, requested: false })
      .text,
    "Cowboy cowboy-v1352 · 99%",
  );
});

Deno.test("a press names what a second press would do, not what the first did", () => {
  // The control's meaning inverts once it has been pressed. Leaving it
  // reading "Reload" would make the cancel a trap.
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "downloading", progress: 0.4, requested: true }),
    { text: "Reloading when ready · 40%", action: "Cancel" },
  );
});

Deno.test("every pressable phase draws its action as an action", () => {
  // The bar looks like the notice it used to be, so the verb has to leave the
  // sentence and become a control of its own.
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "ready" }),
    { text: "cowboy-v1352 is ready", action: "Reload" },
  );
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "ready", secs: 3 }),
    { text: "cowboy-v1352 is ready · 3s", action: "Reload" },
  );
  assertEquals(
    mobileUpdateBanner(undefined, { kind: "ready", secs: -2 }),
    { text: "the new version is ready · 0s", action: "Reload" },
  );
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "rejected" }),
    { text: "cowboy-v1352 didn't start", action: "Try again" },
  );
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "failed", requested: false }),
    { text: "Download paused", action: "Retry" },
  );
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "failed", requested: true }),
    { text: "Download paused · retrying", action: "Retry" },
  );
});

Deno.test("the swap itself offers nothing to press", () => {
  assertEquals(
    mobileUpdateBanner("cowboy-v1352", { kind: "reloading" }),
    { text: "Updating to cowboy-v1352…" },
  );
  assertEquals(mobileUpdateBanner(undefined, { kind: "reloading" }), { text: "Updating…" });
});

Deno.test("a screen reader hears one control, not a layout", () => {
  assertEquals(
    mobileUpdateAnnouncement(mobileUpdateBanner("cowboy-v1352", { kind: "ready" })),
    "cowboy-v1352 is ready. Reload",
  );
  assertEquals(
    mobileUpdateAnnouncement(mobileUpdateBanner("cowboy-v1352", { kind: "reloading" })),
    "Updating to cowboy-v1352…",
  );
});
Deno.test("the mobile bar reads the version and keeps its hooks unconditional", async () => {
  const bannerSource = await Deno.readTextFile(
    new URL("./MobileConnectionBanner.tsx", import.meta.url),
  );
  assertEquals(
    bannerSource.includes("mobileUpdateBanner(readyVersion, phase)"),
    true,
  );
  assertEquals(bannerSource.includes("registration?.waiting?.scriptURL"), true);
  const hooksEnd = bannerSource.indexOf("}, [update.phase, update.streamed]);");
  const earlyReturn = bannerSource.indexOf("if (!banner) return null;");
  assertEquals(hooksEnd > 0 && earlyReturn > hooksEnd, true);
  assertEquals(bannerSource.indexOf("useAutoUpdate(store") < earlyReturn, true);
});

Deno.test("a waiting worker script wins over the current /sw.js", async () => {
  const version = await fetchReadyCowboyVersion(
    (url) =>
      Promise.resolve(
        url.includes("waiting")
          ? 'const VERSION = "cowboy-v1353";'
          : 'const VERSION = "cowboy-v1352";',
      ),
    "https://cowboy.example/sw.js?waiting",
  );
  assertEquals(version, "cowboy-v1353");
});
