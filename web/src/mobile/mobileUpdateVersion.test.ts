import { assertEquals } from "jsr:@std/assert";
import {
  cowboyVersionFromServiceWorkerSource,
  fetchReadyCowboyVersion,
  mobileUpdateBannerLabel,
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

Deno.test("the phone bar narrates the download it is filling with", () => {
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "downloading", progress: 0.62, requested: false }),
    "Cowboy cowboy-v1352 · 62%",
  );
  // The asset count is not known until the deployed index is parsed.
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "downloading", requested: false }),
    "Cowboy cowboy-v1352 · downloading…",
  );
  assertEquals(
    mobileUpdateBannerLabel(undefined, { kind: "downloading", progress: 0.2, requested: false }),
    "New Cowboy version · 20%",
  );
  // Never 100% before the bits are here, however the fraction rounds.
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "downloading", progress: 0.999, requested: false }),
    "Cowboy cowboy-v1352 · 99%",
  );
});

Deno.test("a press taken mid-download is answered, not queued silently", () => {
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "downloading", progress: 0.4, requested: true }),
    "Reloading when ready · 40%",
  );
});

Deno.test("once the build is here the label is the verb", () => {
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "ready" }),
    "Reload to cowboy-v1352",
  );
  // The automatic countdown rides along only while it is really running; a
  // parked one has nothing to say now that the press is right there.
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "ready", secs: 3 }),
    "Reload to cowboy-v1352 · 3s",
  );
  assertEquals(
    mobileUpdateBannerLabel(undefined, { kind: "ready", secs: -2 }),
    "Reload to the new version · 0s",
  );
});

Deno.test("the last two phases name what is happening to this build", () => {
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "reloading" }),
    "Updating to cowboy-v1352…",
  );
  assertEquals(
    mobileUpdateBannerLabel(undefined, { kind: "reloading" }),
    "Updating…",
  );
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "failed", requested: false }),
    "Download paused · tap to retry",
  );
  // A press outlives the attempt that failed: the user asked once and is owed
  // the update, not a second prompt to ask again.
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "failed", requested: true }),
    "Download paused · retrying, then reloading",
  );
});

Deno.test("the mobile bar reads the version and keeps its hooks unconditional", async () => {
  const bannerSource = await Deno.readTextFile(
    new URL("./MobileConnectionBanner.tsx", import.meta.url),
  );
  assertEquals(
    bannerSource.includes("mobileUpdateBannerLabel(readyVersion, phase)"),
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
