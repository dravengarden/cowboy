import { assert, assertEquals } from "jsr:@std/assert";

const repoRoot = new URL("../../", import.meta.url);

async function bytes(path: string): Promise<Uint8Array> {
  return await Deno.readFile(new URL(path, repoRoot));
}

async function pngSize(path: string): Promise<[number, number]> {
  const data = await bytes(path);
  assertEquals([...data.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  return [view.getUint32(16), view.getUint32(20)];
}

async function assertSame(left: string, right: string): Promise<void> {
  assertEquals(await bytes(left), await bytes(right));
}

async function sha256(path: string): Promise<string> {
  const data = await bytes(path);
  const digest = await crypto.subtle.digest(
    "SHA-256",
    data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer,
  );
  return [...new Uint8Array(digest)]
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("");
}

Deno.test("Cowboy 1703 brand assets cover every app surface", async () => {
  const expectedPngSizes: Record<string, [number, number]> = {
    "assets/brand/cowboy-logo.png": [1024, 1024],
    "web/public/cowboy-app-icon-180-v4.png": [180, 180],
    "web/public/cowboy-app-icon-192-v4.png": [192, 192],
    "web/public/cowboy-app-icon-512-v4.png": [512, 512],
    "web/public/cowboy-app-icon-maskable-512-v4.png": [512, 512],
    "apps/native-shell/tauri/icons/icon.png": [512, 512],
    "apps/native-shell/tauri/icons/ios/AppIcon-512@2x.png": [1024, 1024],
  };
  for (const [path, size] of Object.entries(expectedPngSizes)) {
    assertEquals(await pngSize(path), size, path);
  }

  for (const alias of [
    "web/public/cowboy-app-icon-180.png",
    "web/public/cowboy-app-icon-180-v2.png",
    "web/public/cowboy-app-icon-180-v3.png",
    "web/public/apple-touch-icon.png",
    "web/public/apple-touch-icon-precomposed.png",
    "web/public/apple-touch-icon-180x180.png",
    "web/public/apple-touch-icon-180x180-precomposed.png",
  ]) {
    await assertSame("web/public/cowboy-app-icon-180-v4.png", alias);
  }
  await assertSame(
    "web/public/apple-touch-icon-152x152.png",
    "web/public/apple-touch-icon-152x152-precomposed.png",
  );
  await assertSame(
    "web/public/apple-touch-icon-167x167.png",
    "web/public/apple-touch-icon-167x167-precomposed.png",
  );
  for (const alias of [
    "web/public/cowboy-app-icon-192.png",
    "web/public/icon-192.png",
  ]) {
    await assertSame("web/public/cowboy-app-icon-192-v4.png", alias);
  }
  for (const alias of [
    "web/public/cowboy-app-icon-512-v3.png",
    "web/public/cowboy-app-icon-512.png",
    "web/public/cowboy-app-icon-maskable-512-v4.png",
    "web/public/cowboy-app-icon-maskable-512.png",
    "web/public/icon-512.png",
    "web/public/maskable-512.png",
  ]) {
    await assertSame("web/public/cowboy-app-icon-512-v4.png", alias);
  }

  const ico = await bytes("web/public/cowboy-favicon-v4.ico");
  assertEquals([...ico.subarray(0, 4)], [0, 0, 1, 0]);
  await assertSame("web/public/cowboy-favicon-v4.ico", "web/public/cowboy-favicon-v3.ico");
  await assertSame("web/public/cowboy-favicon-v4.ico", "web/public/favicon.ico");
  const icns = await bytes("apps/macos-installer/Resources/Cowboy.icns");
  assertEquals(new TextDecoder().decode(icns.subarray(0, 4)), "icns");
  await assertSame(
    "apps/native-shell/tauri/icons/ios/AppIcon-512@2x.png",
    "apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png",
  );
  await assertSame(
    "apps/native-shell/tauri/icons/icon.icns",
    "apps/macos-installer/Resources/Cowboy.icns",
  );

  const pinned1703Assets: Record<string, string> = {
    "assets/brand/cowboy-logo.png":
      "dc61f7d5a82cf3ab76bb8d65f0ee9bd776e8180d5dc31f7fd70337fa4a74a4df",
    "web/public/cowboy-app-icon-512-v4.png":
      "a496fdd0fee4ad530c7c83dc41913ad6f72f63be19479df32e70abec54b5cbb0",
    "apps/native-shell/tauri/icons/icon.png":
      "c62a258a4bb3b2d8dcb4f9fcfe126b279e4f5334f1f0c79cdad99cbf8bee16bf",
    "apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png":
      "de7cce704bc95355ee8e4cee002c046d8f70d752224f248d0a235745ad2aaebd",
    "apps/macos-installer/Resources/Cowboy.icns":
      "b6b1fb80bd12fb557d8571ce61eec3d2f9e917dab3bfce8b3f43fcd198c2bc6d",
  };
  for (const [path, expected] of Object.entries(pinned1703Assets)) {
    assertEquals(await sha256(path), expected, path);
  }
});

Deno.test("README, PWA, notifications, and Manager reference Cowboy 1703", async () => {
  const read = async (path: string): Promise<string> =>
    await Deno.readTextFile(new URL(path, repoRoot));
  const readme = await read("README.md");
  const index = await read("web/index.html");
  const admin = await read("web/admin.html");
  const manifest = await read("web/public/manifest.webmanifest");
  const serviceWorker = await read("web/public/sw.js");
  const nativeRelease = await read("web/public/native-release.json");
  const managerInfo = await read("apps/macos-installer/Resources/Info.plist");
  const managerBuild = await read("apps/macos-installer/scripts/build-app.sh");

  assert(readme.includes("site/assets/cowboy-readme-mark-light-v2.png"));
  assert(readme.includes("site/assets/cowboy-readme-mark-dark-v2.png"));
  assert(index.includes("/manifest.webmanifest?v=cowboy-v1623"));
  assert(index.includes("/cowboy-favicon-v4.ico"));
  assert(index.includes("/cowboy-app-icon-512-v4.png"));
  assert(index.includes("/cowboy-app-icon-180-v4.png"));
  assert(admin.includes("/cowboy-favicon-v4.ico"));
  assert(admin.includes("/cowboy-app-icon-512-v4.png"));
  assert(manifest.includes('"src": "/cowboy-app-icon-180-v4.png"'));
  assert(manifest.includes('"src": "/cowboy-app-icon-192-v4.png"'));
  assert(manifest.includes('"src": "/cowboy-app-icon-512-v4.png"'));
  assert(manifest.includes('"src": "/cowboy-app-icon-maskable-512-v4.png"'));
  assert(serviceWorker.includes('const VERSION = "cowboy-v1629"'));
  assert(serviceWorker.includes('icon: "/cowboy-app-icon-192-v4.png"'));
  assert(nativeRelease.includes('"latest_version": "0.1.27"'));
  assert(nativeRelease.includes("transcript scrollable"));
  assert(managerInfo.includes("<string>Cowboy.icns</string>"));
  assert(managerBuild.includes("Resources/Cowboy.icns"));
});
