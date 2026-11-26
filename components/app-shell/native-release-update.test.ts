import { test } from "bun:test";
import { nativeReleaseChannelsFor } from "./native-release-channels.ts";

function assertEqual(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${JSON.stringify(actual)} !== ${JSON.stringify(expected)}`);
  }
}

const testflight = { kind: "testflight" as const, url: "https://testflight.apple.com/" };
const appStore = { kind: "app_store" as const, url: "https://apps.apple.com/app/id0" };

test("store channels serve the iOS shell only, unless declared otherwise", () => {
  assertEqual(nativeReleaseChannelsFor([testflight, appStore], "ios"), [testflight, appStore]);
  assertEqual(nativeReleaseChannelsFor([testflight, appStore], "macos"), []);
  assertEqual(nativeReleaseChannelsFor([testflight, appStore], "other"), []);
  const mac = { kind: "app_store" as const, url: "itms-apps://x", platforms: ["macos" as const] };
  assertEqual(nativeReleaseChannelsFor([testflight, mac], "macos"), [mac]);
});
