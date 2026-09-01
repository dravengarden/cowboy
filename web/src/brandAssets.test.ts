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

Deno.test("Cowboy 1412 brand assets cover every app surface", async () => {
  const expectedPngSizes: Record<string, [number, number]> = {
    "assets/brand/cowboy-logo.png": [1024, 1024],
    "web/public/cowboy-app-icon-180-v3.png": [180, 180],
    "web/public/cowboy-app-icon-192.png": [192, 192],
    "web/public/cowboy-app-icon-512-v3.png": [512, 512],
    "apps/native-shell/tauri/icons/icon.png": [512, 512],
    "apps/native-shell/tauri/icons/ios/AppIcon-512@2x.png": [1024, 1024],
  };
  for (const [path, size] of Object.entries(expectedPngSizes)) {
    assertEquals(await pngSize(path), size, path);
  }

  for (const alias of [
    "web/public/cowboy-app-icon-180.png",
    "web/public/cowboy-app-icon-180-v2.png",
    "web/public/apple-touch-icon.png",
    "web/public/apple-touch-icon-precomposed.png",
    "web/public/apple-touch-icon-180x180.png",
    "web/public/apple-touch-icon-180x180-precomposed.png",
  ]) {
    await assertSame("web/public/cowboy-app-icon-180-v3.png", alias);
  }
  await assertSame("web/public/cowboy-app-icon-192.png", "web/public/icon-192.png");
  for (const alias of [
    "web/public/cowboy-app-icon-512.png",
    "web/public/cowboy-app-icon-maskable-512.png",
    "web/public/icon-512.png",
  ]) {
    await assertSame("web/public/cowboy-app-icon-512-v3.png", alias);
  }

  const ico = await bytes("web/public/cowboy-favicon-v3.ico");
  assertEquals([...ico.subarray(0, 4)], [0, 0, 1, 0]);
  const icns = await bytes("apps/macos-installer/Resources/Cowboy.icns");
  assertEquals(new TextDecoder().decode(icns.subarray(0, 4)), "icns");

  const pinned1412Assets: Record<string, string> = {
    "assets/brand/cowboy-logo.png":
      "abb0b21da501300846893286a35b87fcd0c79c9b67f3a289d9d8c8959f7902cc",
    "web/public/cowboy-app-icon-512-v3.png":
      "31878cf551d7557e9320c760a1cb6b173fc18b57171bda7dc117326106069a40",
    "apps/native-shell/tauri/icons/icon.png":
      "5947df902bce7a484e01e6e741c607c6cd8812901cd7bcda9871c06eba2c3729",
    "apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset/AppIcon-512@2x.png":
      "4dd2f9245e514543e57f105a946e43b2beac3a2127768d6c5caaa83f85b693d9",
    "apps/macos-installer/Resources/Cowboy.icns":
      "714ead5ff4c2706b4308eadea96ff4c9435397a980696e565280db23cff1408b",
  };
  for (const [path, expected] of Object.entries(pinned1412Assets)) {
    assertEquals(await sha256(path), expected, path);
  }
});

Deno.test("README, PWA, notifications, and Manager reference Cowboy 1412", async () => {
  const read = async (path: string): Promise<string> =>
    await Deno.readTextFile(new URL(path, repoRoot));
  const readme = await read("README.md");
  const index = await read("web/index.html");
  const admin = await read("web/admin.html");
  const manifest = await read("web/public/manifest.webmanifest");
  const serviceWorker = await read("web/public/sw.js");
  const managerInfo = await read("apps/macos-installer/Resources/Info.plist");
  const managerBuild = await read("apps/macos-installer/scripts/build-app.sh");

  assert(readme.includes("web/public/cowboy-app-icon-512-v3.png"));
  assert(index.includes("/manifest.webmanifest?v=cowboy-v1617"));
  assert(index.includes("/cowboy-favicon-v3.ico"));
  assert(index.includes("/cowboy-app-icon-512-v3.png"));
  assert(index.includes("/cowboy-app-icon-180-v3.png"));
  assert(admin.includes("/cowboy-favicon-v3.ico"));
  assert(admin.includes("/cowboy-app-icon-512-v3.png"));
  assert(manifest.includes('"src": "/cowboy-app-icon-180-v3.png"'));
  assert(manifest.includes('"src": "/cowboy-app-icon-512.png"'));
  assert(serviceWorker.includes('const VERSION = "cowboy-v1617"'));
  assert(serviceWorker.includes('icon: "/cowboy-app-icon-192.png"'));
  assert(managerInfo.includes("<string>Cowboy.icns</string>"));
  assert(managerBuild.includes("Resources/Cowboy.icns"));
});
