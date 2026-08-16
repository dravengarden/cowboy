import { assertEquals } from "jsr:@std/assert";
import {
  cowboyVersionFromServiceWorkerSource,
  fetchReadyCowboyVersion,
  mobileUpdateBannerLabel,
} from "./mobileUpdateVersion.ts";

Deno.test("the update banner names the ready service-worker version", () => {
  assertEquals(
    cowboyVersionFromServiceWorkerSource('const VERSION = "cowboy-v1352";'),
    "cowboy-v1352",
  );
  assertEquals(
    cowboyVersionFromServiceWorkerSource("const VERSION = 'cowboy-v12';"),
    "cowboy-v12",
  );
  assertEquals(cowboyVersionFromServiceWorkerSource("const ASSET_CACHE"), undefined);
  assertEquals(
    mobileUpdateBannerLabel("cowboy-v1352"),
    "New Cowboy version cowboy-v1352 ready",
  );
  assertEquals(mobileUpdateBannerLabel(undefined), "New Cowboy version ready");
});

Deno.test("the mobile banner reads the ready service-worker version", async () => {
  const bannerSource = await Deno.readTextFile(
    new URL("./MobileConnectionBanner.tsx", import.meta.url),
  );
  assertEquals(bannerSource.includes("mobileUpdateBannerLabel(readyVersion)"), true);
  assertEquals(bannerSource.includes("registration?.waiting?.scriptURL"), true);
  const hooksEnd = bannerSource.indexOf("}, [isUpdate]);");
  const earlyReturn = bannerSource.indexOf("if (!banner) return null;");
  assertEquals(hooksEnd > 0 && earlyReturn > hooksEnd, true);
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
