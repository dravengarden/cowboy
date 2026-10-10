import { test } from "bun:test";
import { nativeReleaseChannelsFor } from "./native-release-channels.ts";

function assertEqual(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${JSON.stringify(actual)} !== ${JSON.stringify(expected)}`);
  }
}

const sidestore = { kind: "sidestore" as const, url: "sidestore://source" };

test("store channels serve the iOS shell only, unless declared otherwise", () => {
  assertEqual(nativeReleaseChannelsFor([sidestore], "ios"), [sidestore]);
  assertEqual(nativeReleaseChannelsFor([sidestore], "macos"), []);
  assertEqual(nativeReleaseChannelsFor([sidestore], "other"), []);
  const mac = { kind: "app_store" as const, url: "itms-apps://x", platforms: ["macos" as const] };
  assertEqual(nativeReleaseChannelsFor([sidestore, mac], "macos"), [mac]);
});
