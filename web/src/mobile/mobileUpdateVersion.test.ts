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

Deno.test("every phone update label narrates, never asks", () => {
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "counting", secs: 3 }),
    "New Cowboy version cowboy-v1352 · updating in 3s",
  );
  assertEquals(
    mobileUpdateBannerLabel(undefined, { kind: "counting", secs: 0 }),
    "New Cowboy version · updating in 0s",
  );
  // A negative counter can never reach the copy.
  assertEquals(
    mobileUpdateBannerLabel(undefined, { kind: "counting", secs: -2 }),
    "New Cowboy version · updating in 0s",
  );
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "held" }),
    "New Cowboy version cowboy-v1352 ready · updating when you pause",
  );
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "applying" }),
    "Updating to cowboy-v1352…",
  );
  assertEquals(
    mobileUpdateBannerLabel(undefined, { kind: "applying" }),
    "Downloading the update…",
  );
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352", { kind: "failed" }),
    "The update could not be downloaded yet · retrying",
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
  const hooksEnd = bannerSource.indexOf("}, [isUpdate]);");
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
